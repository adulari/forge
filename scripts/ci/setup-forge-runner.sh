#!/usr/bin/env bash
set -euo pipefail

# --- Config ---------------------------------------------------------------
OWNER_REPO="adulari/forge"
RUNNER_USER="github-runner"
RUNNER_HOME="/home/github-runner"
LABELS="self-hosted,linux,x64"
# The stock actions-runner agent runs exactly one job at a time. Every `[self-hosted, linux,
# x64]` job across ci.yml + security.yml was measured queueing on a single runner instance —
# real PRs (#559/#560) took ~8m45s wall clock for jobs whose actual work summed to ~5m51s and
# whose longest single job was ~2m24s. Registering N runner instances (same labels, separate
# work dirs) lets GitHub actually schedule them concurrently instead of serializing.
NUM_RUNNERS="${NUM_RUNNERS:-3}"

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

echo "==> Sizing sandbox limits for this machine (split across $NUM_RUNNERS runner instances)"
CPUS=$(nproc)
CPUQUOTA_TOTAL=$(( CPUS * 100 * 3 / 8 )) # ~37.5% of cores total -> 1200% on 32c
MEM_GB=$(free -g | awk '/^Mem:/{print $2}')
MEM_MAX_TOTAL=$(( MEM_GB * 2 / 5 )) # ~40% of RAM total -> 12 on 30G
if [ "$MEM_MAX_TOTAL" -lt 4 ]; then
  MEM_MAX_TOTAL=4
fi
CPUQUOTA=$(( CPUQUOTA_TOTAL / NUM_RUNNERS ))
if [ "$CPUQUOTA" -lt 100 ]; then
  CPUQUOTA=100 # floor of 1 core each — don't starve individual jobs on small boxes
fi
MEM_MAX=$(( MEM_MAX_TOTAL / NUM_RUNNERS ))
if [ "$MEM_MAX" -lt 2 ]; then
  MEM_MAX=2
fi

SVC_NAMES=()

TARBALL_CACHE="/tmp/actions-runner-linux-x64.tar.gz"
if [ ! -f "$TARBALL_CACHE" ]; then
  echo "==> Fetching latest actions/runner release (shared across all $NUM_RUNNERS instances)"
  # Use gh (authed, no rate limit). Piping raw curl into `grep -m1` closes the pipe early,
  # giving curl a SIGPIPE write error (curl exit 23) that `set -o pipefail` turns fatal.
  LATEST_TAG="$(sudo -u "$RUN_USER" gh api repos/actions/runner/releases/latest -q .tag_name | sed 's/^v//')"
  if [ -z "$LATEST_TAG" ]; then
    echo "error: could not determine latest actions/runner version" >&2
    exit 1
  fi
  TARBALL="actions-runner-linux-x64-${LATEST_TAG}.tar.gz"
  echo "==> Downloading $TARBALL"
  curl -fSL -o "$TARBALL_CACHE" \
    "https://github.com/actions/runner/releases/download/v${LATEST_TAG}/${TARBALL}"
else
  echo "==> Reusing already-downloaded runner tarball at $TARBALL_CACHE"
fi

for i in $(seq 1 "$NUM_RUNNERS"); do
  RUNNER_DIR="$RUNNER_HOME/actions-runner-$i"
  RUNNER_NAME="$(uname -n)-$i"
  echo
  echo "==> [$i/$NUM_RUNNERS] Setting up $RUNNER_NAME at $RUNNER_DIR"

  mkdir -p "$RUNNER_DIR"
  if [ ! -f "$RUNNER_DIR/config.sh" ]; then
    tar xzf "$TARBALL_CACHE" -C "$RUNNER_DIR"
  else
    echo "  runner already extracted — reconfigure still runs below"
  fi
  chown -R "$RUNNER_USER:$RUNNER_USER" "$RUNNER_DIR"

  echo "  minting registration token as $RUN_USER"
  RUNNER_TOKEN="$(sudo -u "$RUN_USER" gh api -X POST "repos/$OWNER_REPO/actions/runners/registration-token" -q .token)"
  if [ -z "$RUNNER_TOKEN" ]; then
    echo "error: empty registration token. Ensure 'gh auth status' works for $RUN_USER." >&2
    exit 1
  fi

  echo "  configuring runner (--replace makes this safe to re-run)"
  (
    cd "$RUNNER_DIR"
    sudo -u "$RUNNER_USER" ./config.sh --url "https://github.com/$OWNER_REPO" --token "$RUNNER_TOKEN" \
      --name "$RUNNER_NAME" --labels "$LABELS" --work _work --replace --unattended
  )

  echo "  installing + starting as a system service"
  if [ ! -f "$RUNNER_DIR/.service" ]; then
    (cd "$RUNNER_DIR" && ./svc.sh install "$RUNNER_USER")
  fi
  (cd "$RUNNER_DIR" && ./svc.sh start)
  SVC_NAME="$(cat "$RUNNER_DIR/.service")"

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
  echo "  $RUNNER_NAME -> $SVC_NAME (CPUQuota=${CPUQUOTA}% MemoryMax=${MEM_MAX}G)"
  SVC_NAMES+=("$SVC_NAME")
done

rm -f "$TARBALL_CACHE"

echo "==> Installing weekly disk-cleanup timer across all instances (disk is the tightest resource on this box, not docker — none of our workflows use it)"
cat > /etc/systemd/system/forge-runner-cleanup.service <<EOF
[Unit]
Description=Prune stale forge CI runner build artifacts

[Service]
Type=oneshot
ExecStart=/usr/bin/env bash -c 'find "$RUNNER_HOME"/actions-runner-*/_work -maxdepth 4 -type d -name target -mtime +14 -exec rm -rf {} + ; find "$RUNNER_HOME"/actions-runner-*/_work/_temp -mindepth 1 -mtime +7 -delete 2>/dev/null || true'
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
echo "Runner instances: $NUM_RUNNERS, each CPUQuota=${CPUQUOTA}% MemoryMax=${MEM_MAX}G Nice=10 (best-effort IO)"
echo "Cleanup timer:    forge-runner-cleanup.timer (weekly, prunes _work targets >14d / _temp >7d)"
echo
echo "Verify with:"
for svc in "${SVC_NAMES[@]}"; do
  echo "  systemctl status \"$svc\" --no-pager"
done
echo "  sudo -u \"$RUN_USER\" gh api repos/$OWNER_REPO/actions/runners -q '.runners[].name'"
echo "=================================================================="
