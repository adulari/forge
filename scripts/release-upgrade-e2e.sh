#!/usr/bin/env bash
# Release-boundary E2E for the self-hosted Arch Linux release gate.
#
# Installs the latest public Forge in a completely isolated HOME, creates real mock-provider
# history plus a daemon session, swaps in the candidate, reconnects/resumes both, rolls back to
# prove DB compatibility, then reinstalls the candidate. Nothing touches the operator's config,
# keyring, sessions, daemon, or telemetry.
#
#   scripts/release-upgrade-e2e.sh [base-tag] [candidate-binary]
#   scripts/release-upgrade-e2e.sh v2.6.4 target/release/forge
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CANDIDATE="${2:-$REPO/target/release/forge}"
BASE_TAG="${1:-}"

[[ "$(uname -s)" == Linux && "$(uname -m)" == x86_64 ]] || {
  echo "release-upgrade-e2e: only the x86_64 Linux release leg is supported" >&2
  exit 2
}
[[ -x "$CANDIDATE" ]] || {
  echo "release-upgrade-e2e: candidate binary is missing: $CANDIDATE" >&2
  exit 2
}

latest_public_release_tag() {
  local tag=""

  if command -v gh >/dev/null 2>&1; then
    tag="$(gh release view --repo Adulari/forge --json tagName --jq .tagName 2>/dev/null || true)"
    if [[ "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
      printf '%s\n' "$tag"
      return 0
    fi
    echo "release-upgrade-e2e: authenticated release lookup failed; trying public API" >&2
  fi

  tag="$(
    curl --fail --silent --show-error --location \
      --header 'Accept: application/vnd.github+json' \
      --header 'User-Agent: forge-release-upgrade-e2e' \
      https://api.github.com/repos/Adulari/forge/releases/latest \
      | python3 -c 'import json, sys; print(json.load(sys.stdin).get("tag_name", ""))'
  )" || return 1
  [[ "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || return 1
  printf '%s\n' "$tag"
}

public_release_tags() {
  local tags=""

  if command -v gh >/dev/null 2>&1; then
    tags="$(
      gh release list --repo Adulari/forge --limit 50 --json tagName,isDraft,isPrerelease \
        --jq '.[] | select(.isDraft == false and .isPrerelease == false) | .tagName' \
        2>/dev/null || true
    )"
    if grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+$' <<<"$tags"; then
      grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' <<<"$tags"
      return 0
    fi
    echo "release-upgrade-e2e: authenticated release list failed; trying public API" >&2
  fi

  tags="$(
    curl --fail --silent --show-error --location \
      --header 'Accept: application/vnd.github+json' \
      --header 'User-Agent: forge-release-upgrade-e2e' \
      'https://api.github.com/repos/Adulari/forge/releases?per_page=50' \
      | python3 -c '
import json, re, sys
for release in json.load(sys.stdin):
    tag = release.get("tag_name", "")
    if not release.get("draft") and not release.get("prerelease") and re.fullmatch(r"v\d+\.\d+\.\d+", tag):
        print(tag)
'
  )" || return 1
  [[ -n "$tags" ]] || return 1
  printf '%s\n' "$tags"
}

if [[ -z "$BASE_TAG" ]]; then
  BASE_TAG="$(latest_public_release_tag)" || {
    echo "release-upgrade-e2e: could not discover the latest public release" >&2
    exit 1
  }
fi
[[ "$BASE_TAG" == v* ]] || BASE_TAG="v$BASE_TAG"

CANDIDATE_VERSION="$($CANDIDATE --version | awk '{print $NF}')"
if [[ "v$CANDIDATE_VERSION" == "$BASE_TAG" ]]; then
  echo "release-upgrade-e2e: candidate v$CANDIDATE_VERSION is already public; nothing to upgrade"
  exit 0
fi

ROOT="$(mktemp -d "${TMPDIR:-/tmp}/forge-release-upgrade.XXXXXX")"
HOME_DIR="$ROOT/home"
CFG="$HOME_DIR/.config/forge"
DATA="$HOME_DIR/.local/share/forge"
BIN="$HOME_DIR/.local/bin"
PROJECT="$ROOT/project"
DOWNLOAD="$ROOT/download"
DAEMON_PID=""
mkdir -p "$CFG" "$DATA" "$BIN" "$PROJECT" "$DOWNLOAD"

stop_daemon() {
  if [[ -n "$DAEMON_PID" ]] && kill -0 "$DAEMON_PID" 2>/dev/null; then
    kill -INT "$DAEMON_PID" 2>/dev/null || true
    for _ in {1..50}; do
      kill -0 "$DAEMON_PID" 2>/dev/null || break
      sleep 0.1
    done
    kill -TERM "$DAEMON_PID" 2>/dev/null || true
    wait "$DAEMON_PID" 2>/dev/null || true
  fi
  DAEMON_PID=""
}
cleanup() {
  stop_daemon
  if [[ "${KEEP_ARTIFACTS:-0}" == 1 ]]; then
    echo "release-upgrade-e2e: kept scratch at $ROOT" >&2
  else
    rm -rf "$ROOT"
  fi
}
failure() {
  code=$?
  echo "release-upgrade-e2e: FAILED (scratch: $ROOT)" >&2
  [[ -f "$ROOT/daemon.log" ]] && tail -80 "$ROOT/daemon.log" >&2 || true
  exit "$code"
}
trap failure ERR
trap cleanup EXIT

forge_env() {
  env \
    HOME="$HOME_DIR" \
    XDG_CONFIG_HOME="$HOME_DIR/.config" \
    XDG_DATA_HOME="$HOME_DIR/.local/share" \
    FORGE_NO_UPDATE_CHECK=1 \
    FORGE_TELEMETRY_FORCE=1 \
    FORGE_POSTHOG_KEY=release-upgrade-e2e \
    FORGE_POSTHOG_HOST=http://127.0.0.1:9 \
    "$@"
}

wait_http() {
  url=$1
  for _ in {1..100}; do
    curl --fail --silent "$url" >/dev/null 2>&1 && return 0
    kill -0 "$DAEMON_PID" 2>/dev/null || return 1
    sleep 0.1
  done
  return 1
}

start_daemon() {
  binary=$1
  port=$2
  : > "$ROOT/daemon.log"
  (
    cd "$PROJECT"
    exec env \
      HOME="$HOME_DIR" \
      XDG_CONFIG_HOME="$HOME_DIR/.config" \
      XDG_DATA_HOME="$HOME_DIR/.local/share" \
      FORGE_NO_UPDATE_CHECK=1 \
      FORGE_TELEMETRY_FORCE=1 \
      FORGE_POSTHOG_KEY=release-upgrade-e2e \
      FORGE_POSTHOG_HOST=http://127.0.0.1:9 \
      "$binary" serve --local --mock --port "$port"
  ) >"$ROOT/daemon.log" 2>&1 &
  DAEMON_PID=$!
  for _ in {1..100}; do
    [[ -s "$CFG/serve-token" ]] && break
    kill -0 "$DAEMON_PID" 2>/dev/null || return 1
    sleep 0.1
  done
  [[ -s "$CFG/serve-token" ]]
  TOKEN="$(tr -d '\r\n' < "$CFG/serve-token")"
  BASE_URL="http://127.0.0.1:$port/$TOKEN"
  wait_http "$BASE_URL/api/sessions"
}

json_id() {
  python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])'
}

contains_id() {
  file=$1
  wanted=$2
  python3 - "$file" "$wanted" <<'PY'
import json, sys
rows = json.load(open(sys.argv[1]))
assert any(row.get("id") == sys.argv[2] for row in rows), sys.argv[2]
PY
}

echo "release-upgrade-e2e: $BASE_TAG -> v$CANDIDATE_VERSION"
ASSET=forge-x86_64-unknown-linux-gnu.tar.gz
curl --fail --silent --show-error --location \
  "https://github.com/Adulari/forge/releases/download/$BASE_TAG/$ASSET" \
  --output "$DOWNLOAD/$ASSET"
curl --fail --silent --show-error --location \
  "https://github.com/Adulari/forge/releases/download/$BASE_TAG/checksums.txt" \
  --output "$DOWNLOAD/checksums.txt"
EXPECTED="$(awk -v asset="$ASSET" '$2 == asset { print $1 }' "$DOWNLOAD/checksums.txt")"
[[ -n "$EXPECTED" ]]
printf '%s  %s\n' "$EXPECTED" "$DOWNLOAD/$ASSET" | sha256sum --check --status
tar -xzf "$DOWNLOAD/$ASSET" -C "$DOWNLOAD"
OLD="$DOWNLOAD/forge-x86_64-unknown-linux-gnu/forge"
[[ -f "$OLD" ]]
# GitHub's artifact transport historically stripped the executable bit before release packaging.
# The installer repairs it with `install -m 0755`; keep old releases testable while the current
# release workflow now preserves the bit in its archives too.
chmod 0755 "$OLD"
[[ "$($OLD --version | awk '{print $NF}')" == "${BASE_TAG#v}" ]]

cat > "$CFG/config.toml" <<'EOF'
[telemetry]
enabled = true

[update]
check = false
EOF
printf 'release-upgrade-secret-sentinel\n' > "$CFG/secret.key"
CFG_SHA="$(sha256sum "$CFG/config.toml" | awk '{print $1}')"
SECRET_SHA="$(sha256sum "$CFG/secret.key" | awk '{print $1}')"

# Exercise the real self-replacing updater against already-published artifacts: previous public
# release -> current public base. The unreleased candidate itself is swapped in below because it
# does not have a downloadable GitHub asset yet.
SELF_UPDATE_VERIFIED=0
RELEASE_TAGS_OUTPUT="$(public_release_tags)" || {
  echo "release-upgrade-e2e: could not discover public releases" >&2
  exit 1
}
mapfile -t RELEASE_TAGS <<<"$RELEASE_TAGS_OUTPUT"
PREVIOUS_TAG=""
for ((i = 0; i + 1 < ${#RELEASE_TAGS[@]}; i++)); do
  if [[ "${RELEASE_TAGS[$i]}" == "$BASE_TAG" ]]; then
    PREVIOUS_TAG="${RELEASE_TAGS[$((i + 1))]}"
    break
  fi
done
if [[ -n "$PREVIOUS_TAG" ]]; then
  PREVIOUS_DIR="$DOWNLOAD/previous"
  mkdir -p "$PREVIOUS_DIR"
  curl --fail --silent --show-error --location \
    "https://github.com/Adulari/forge/releases/download/$PREVIOUS_TAG/$ASSET" \
    --output "$PREVIOUS_DIR/$ASSET"
  curl --fail --silent --show-error --location \
    "https://github.com/Adulari/forge/releases/download/$PREVIOUS_TAG/checksums.txt" \
    --output "$PREVIOUS_DIR/checksums.txt"
  PREVIOUS_SHA="$(awk -v asset="$ASSET" '$2 == asset { print $1 }' "$PREVIOUS_DIR/checksums.txt")"
  [[ -n "$PREVIOUS_SHA" ]]
  printf '%s  %s\n' "$PREVIOUS_SHA" "$PREVIOUS_DIR/$ASSET" | sha256sum --check --status
  tar -xzf "$PREVIOUS_DIR/$ASSET" -C "$PREVIOUS_DIR"
  install -m 0755 "$PREVIOUS_DIR/forge-x86_64-unknown-linux-gnu/forge" "$BIN/self-update-forge"
  forge_env "$BIN/self-update-forge" update > "$ROOT/self-update.log" 2>&1
  [[ "$(forge_env "$BIN/self-update-forge" --version | awk '{print $NF}')" == "${BASE_TAG#v}" ]]
  SELF_UPDATE_VERIFIED=1
fi

install -m 0755 "$OLD" "$BIN/forge"
(
  cd "$PROJECT"
  forge_env "$BIN/forge" run UPGRADE_OLD_MARKER --mock --mode bypass
) > "$ROOT/old-run.log" 2>&1
forge_env "$BIN/forge" sessions > "$ROOT/old-sessions.txt"
CLI_SESSION="$(grep -Eom1 '[0-9a-f]{8}(-[0-9a-f]{4}){3}-[0-9a-f]{12}' "$ROOT/old-run.log")"
[[ -n "$CLI_SESSION" ]]
forge_env "$BIN/forge" replay "$CLI_SESSION" --json > "$ROOT/old-replay.json"
grep -q UPGRADE_OLD_MARKER "$ROOT/old-replay.json"

PORT="$(python3 - <<'PY'
import socket
s = socket.socket()
s.bind(("127.0.0.1", 0))
print(s.getsockname()[1])
s.close()
PY
)"
start_daemon "$BIN/forge" "$PORT"
OLD_DAEMON_SESSION="$(curl --fail --silent --show-error \
  -H 'content-type: application/json' \
  --data-binary "{\"cwd\":\"$PROJECT\",\"resume\":\"$CLI_SESSION\"}" \
  "$BASE_URL/api/sessions" | json_id)"
[[ "$OLD_DAEMON_SESSION" == "$CLI_SESSION" ]]
curl --fail --silent "$BASE_URL/api/sessions" > "$ROOT/old-active.json"
contains_id "$ROOT/old-active.json" "$OLD_DAEMON_SESSION"
stop_daemon

install -m 0755 "$CANDIDATE" "$BIN/forge"
[[ "$(forge_env "$BIN/forge" --version | awk '{print $NF}')" == "$CANDIDATE_VERSION" ]]
(
  cd "$PROJECT"
  forge_env "$BIN/forge" run UPGRADE_NEW_MARKER --mock --mode bypass --resume "$CLI_SESSION"
) > "$ROOT/new-run.log" 2>&1
forge_env "$BIN/forge" replay "$CLI_SESSION" --json > "$ROOT/new-replay.json"
grep -q UPGRADE_OLD_MARKER "$ROOT/new-replay.json"
grep -q UPGRADE_NEW_MARKER "$ROOT/new-replay.json"

[[ "$(sha256sum "$CFG/config.toml" | awk '{print $1}')" == "$CFG_SHA" ]]
[[ "$(sha256sum "$CFG/secret.key" | awk '{print $1}')" == "$SECRET_SHA" ]]
python3 - "$DATA/anonymous-telemetry.json" <<'PY'
import json, sys
state = json.load(open(sys.argv[1]))
assert state["installed"] is True
installs = [event for event in state["pending"] if event["event"] == "forge_installed"]
assert len(installs) == 1, installs
PY
python3 - "$DATA/forge.db" <<'PY'
import sqlite3, sys
db = sqlite3.connect(sys.argv[1])
assert db.execute("pragma integrity_check").fetchone()[0] == "ok"
PY

start_daemon "$BIN/forge" "$PORT"
curl --fail --silent "$BASE_URL/api/sessions/past?limit=100" > "$ROOT/past.json"
contains_id "$ROOT/past.json" "$OLD_DAEMON_SESSION"
RESUMED="$(curl --fail --silent --show-error \
  -H 'content-type: application/json' \
  --data-binary "{\"cwd\":\"$PROJECT\",\"resume\":\"$OLD_DAEMON_SESSION\"}" \
  "$BASE_URL/api/sessions" | json_id)"
[[ "$RESUMED" == "$OLD_DAEMON_SESSION" ]]
NEW_DAEMON_SESSION="$(curl --fail --silent --show-error \
  -H 'content-type: application/json' \
  --data-binary "{\"cwd\":\"$PROJECT\",\"title\":\"post-upgrade daemon\"}" \
  "$BASE_URL/api/sessions" | json_id)"
[[ "$NEW_DAEMON_SESSION" != "$OLD_DAEMON_SESSION" ]]
curl --fail --silent "$BASE_URL/api/sessions" > "$ROOT/new-active.json"
contains_id "$ROOT/new-active.json" "$OLD_DAEMON_SESSION"
contains_id "$ROOT/new-active.json" "$NEW_DAEMON_SESSION"
stop_daemon

# Roll back the executable only. The previous release must still understand the history touched by
# the candidate; then reinstall the candidate and verify the same state one final time.
install -m 0755 "$OLD" "$BIN/forge"
forge_env "$BIN/forge" replay "$CLI_SESSION" --json > "$ROOT/rollback-replay.json"
grep -q UPGRADE_OLD_MARKER "$ROOT/rollback-replay.json"
grep -q UPGRADE_NEW_MARKER "$ROOT/rollback-replay.json"
install -m 0755 "$CANDIDATE" "$BIN/forge"
forge_env "$BIN/forge" replay "$CLI_SESSION" --json > "$ROOT/final-replay.json"
grep -q UPGRADE_NEW_MARKER "$ROOT/final-replay.json"

trap - ERR
echo "release-upgrade-e2e: PASS"
echo "  config + secret: byte-identical"
[[ "$SELF_UPDATE_VERIFIED" == 1 ]] && echo "  updater: previous public release self-replaced to $BASE_TAG"
echo "  history: resumed across upgrade and rollback"
echo "  daemon: restarted, old session resumed, new session created"
echo "  telemetry: one anonymous install marker preserved"
echo "  database: SQLite integrity_check=ok"
