#!/usr/bin/env bash
set -euo pipefail

# --- Config ---------------------------------------------------------------
OWNER_REPO="adulari/forge"
RUNNER_USER="github-runner"
RUNNER_HOME="/home/github-runner"
RUNNER_DIR="$RUNNER_HOME/actions-runner"
LABELS="self-hosted,linux,x64"

if [ "${EUID:-$(id -u)}" -ne 0 ]; then
  echo "run with: sudo bash scripts/ci/setup-forge-runner.sh" >&2
  exit 1
fi

# The invoking (non-root) user's `gh` auth is what mints the runner registration token —
# root has no gh session of its own.
RUN_USER="${SUDO_USER:-}"
if [ -z "$RUN_USER" ]; then
  echo "error: could not determine invoking user (\$SUDO_USER empty). Run via 'sudo', not as root directly." >&2
  exit 1
fi

echo "==> Installing runner dependencies (Arch, not the bundled Debian installdependencies.sh)"
pacman -S --needed --noconfirm icu krb5 openssl zlib lttng-ust

echo "==> Ensuring $RUNNER_USER exists"
if ! id -u "$RUNNER_USER" &>/dev/null; then
  useradd -m -d "$RUNNER_HOME" -s "$(command -v nologin || echo /usr/sbin/nologin)" "$RUNNER_USER"
fi
# Linger keeps the runner's service processes alive without an active login session.
loginctl enable-linger "$RUNNER_USER"

mkdir -p "$RUNNER_DIR"

if [ ! -f "$RUNNER_DIR/config.sh" ]; then
  echo "==> Fetching latest actions/runner release"
  # Use gh (authed, no rate limit). Piping raw curl into `grep -m1` closes the pipe early,
  # giving curl a SIGPIPE write error (curl exit 23) that `set -o pipefail` turns fatal.
  LATEST_TAG="$(sudo -u "$RUN_USER" gh api repos/actions/runner/releases/latest -q .tag_name | sed 's/^v//')"
  if [ -z "$LATEST_TAG" ]; then
    echo "error: could not determine latest actions/runner version" >&2
    exit 1
  fi
  TARBALL="actions-runner-linux-x64-${LATEST_TAG}.tar.gz"
  echo "==> Downloading $TARBALL"
  # Download + extract as root, then chown. Running curl as the unprivileged runner user
  # can hit a PAM fsize ulimit that truncates the ~60MB tarball mid-write (curl error 23).
  curl -fSL -o "$RUNNER_DIR/$TARBALL" \
    "https://github.com/actions/runner/releases/download/v${LATEST_TAG}/${TARBALL}"
  tar xzf "$RUNNER_DIR/$TARBALL" -C "$RUNNER_DIR"
  rm -f "$RUNNER_DIR/$TARBALL"
else
  echo "==> Runner already extracted at $RUNNER_DIR — skipping download (reconfigure still runs below)"
fi
chown -R "$RUNNER_USER:$RUNNER_USER" "$RUNNER_DIR"

echo "==> Minting registration token as $RUN_USER"
RUNNER_TOKEN="${RUNNER_TOKEN:-$(sudo -u "$RUN_USER" gh api -X POST "repos/$OWNER_REPO/actions/runners/registration-token" -q .token)}"
if [ -z "$RUNNER_TOKEN" ]; then
  echo "error: empty registration token. Ensure 'gh auth status' works for $RUN_USER, or pass RUNNER_TOKEN=... sudo -E bash $0" >&2
  exit 1
fi

echo "==> Configuring runner (--replace makes this safe to re-run)"
cd "$RUNNER_DIR"
sudo -u "$RUNNER_USER" ./config.sh --url "https://github.com/$OWNER_REPO" --token "$RUNNER_TOKEN" \
  --name "$(uname -n)" --labels "$LABELS" --work _work --replace --unattended

echo "==> Installing + starting as a system service"
if [ ! -f "$RUNNER_DIR/.service" ]; then
  ./svc.sh install "$RUNNER_USER"
fi
./svc.sh start
SVC_NAME="$(cat "$RUNNER_DIR/.service")"

echo "==> Sizing sandbox limits for this machine"
CPUS=$(nproc)
CPUQUOTA=$(( CPUS * 100 * 3 / 8 )) # ~37.5% of cores -> 1200% on 32c
MEM_GB=$(free -g | awk '/^Mem:/{print $2}')
MEM_MAX=$(( MEM_GB * 2 / 5 )) # ~40% of RAM -> 12 on 30G
if [ "$MEM_MAX" -lt 4 ]; then
  MEM_MAX=4
fi

DROPIN_DIR="/etc/systemd/system/${SVC_NAME}.d"
mkdir -p "$DROPIN_DIR"
cat > "$DROPIN_DIR/10-limits.conf" <<EOF
[Service]
CPUQuota=${CPUQUOTA}%
MemoryMax=${MEM_MAX}G
Nice=10
IOSchedulingClass=best-effort
IOSchedulingPriority=7
EOF

systemctl daemon-reload
systemctl restart "$SVC_NAME"

echo "==> Installing weekly disk-cleanup timer (disk is the tightest resource on this box, not docker — none of our workflows use it)"
cat > /etc/systemd/system/forge-runner-cleanup.service <<EOF
[Unit]
Description=Prune stale forge CI runner build artifacts

[Service]
Type=oneshot
ExecStart=/usr/bin/env bash -c 'find "$RUNNER_DIR/_work" -maxdepth 4 -type d -name target -mtime +14 -exec rm -rf {} + ; find "$RUNNER_DIR/_work/_temp" -mindepth 1 -mtime +7 -delete 2>/dev/null || true'
EOF

cat > /etc/systemd/system/forge-runner-cleanup.timer <<EOF
[Unit]
Description=Weekly forge CI runner disk cleanup

[Timer]
OnCalendar=weekly
Persistent=true

[Install]
WantedBy=timers.target
EOF

systemctl daemon-reload
systemctl enable --now forge-runner-cleanup.timer

echo
echo "=================================================================="
echo "Runner service:   $SVC_NAME"
echo "Sandbox limits:   CPUQuota=${CPUQUOTA}% MemoryMax=${MEM_MAX}G Nice=10 (best-effort IO)"
echo "Cleanup timer:    forge-runner-cleanup.timer (weekly, prunes _work targets >14d / _temp >7d)"
echo
echo "Verify with:"
echo "  systemctl status \"$SVC_NAME\" --no-pager"
echo "  sudo -u \"$RUN_USER\" gh api repos/$OWNER_REPO/actions/runners -q '.runners[].name'"
echo "=================================================================="
