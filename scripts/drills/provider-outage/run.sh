#!/usr/bin/env bash
# Scripted outage drill: `forge run` against a fake gateway that answers 503 no_capacity for
# DOWN_SECS after the first request, then a healthy reply. Everything lives in a temp root
# (HOME, XDG, FORGE_DB); nothing touches the real daemon, store or config.
#   usage: run.sh [DOWN_SECS=180]
set -u
DOWN_SECS="${1:-180}"
HOT_SECS="${HOT_SECS:-30}"        # mesh.pin_outage_wait_secs: the hot phase, shortened
LONG_SECS="${LONG_SECS:-900}"     # mesh.outage_wait_secs: the long wait
HERE="$(cd "$(dirname "$0")" && pwd)"
WORKTREE="$(cd "$HERE/../../.." && pwd)"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}"
ROOT="$(mktemp -d)"
PORT="$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])')"
GW_PID=""
trap '[ -n "$GW_PID" ] && kill "$GW_PID" 2>/dev/null; rm -rf "$ROOT"' EXIT

mkdir -p "$ROOT/home" "$ROOT/cfg/forge" "$ROOT/data" "$ROOT/ws"
cat > "$ROOT/cfg/forge/config.toml" <<TOML
[[providers.custom]]
namespace = "drill"
base_url  = "http://127.0.0.1:${PORT}/v1"
free      = true
models    = ["flash"]

[mesh]
pin_outage_wait_secs = ${HOT_SECS}
outage_wait_secs = ${LONG_SECS}
TOML

echo "== build"
( cd "$WORKTREE" && cargo build -p forge-agent --bin forge 2>&1 | tail -2 )
FORGE="$(cargo metadata --no-deps --format-version 1 --manifest-path "$WORKTREE/Cargo.toml" | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')/debug/forge"

python3 "$HERE/fake_gateway.py" "$PORT" "$DOWN_SECS" "$ROOT/gateway.jsonl" &
GW_PID=$!
sleep 1
echo "== gateway on 127.0.0.1:$PORT, 503 no_capacity for ${DOWN_SECS}s after the first request"
echo "== hot phase ${HOT_SECS}s, outage_wait_secs ${LONG_SECS}"
START=$(date +%s)
echo "== forge run started $(date +%H:%M:%S)"
( cd "$ROOT/ws" && env -i PATH="$PATH" HOME="$ROOT/home" XDG_CONFIG_HOME="$ROOT/cfg" \
    XDG_DATA_HOME="$ROOT/data" FORGE_DB="$ROOT/forge.db" FORGE_PERMISSION_MODE=default \
    FORGE_AUTOFIX__AUTO_DETECT=false \
    "$FORGE" run -p "say hello" --model drill::flash --verbose 2>&1 < /dev/null \
  | while IFS= read -r line; do printf '[+%03ds] %s\n' "$(( $(date +%s) - START ))" "$line"; done )
RC=${PIPESTATUS[0]}
echo "== forge exited rc=$RC after $(( $(date +%s) - START ))s"

echo "== gateway requests (since_first, down)"
python3 - "$ROOT/gateway.jsonl" <<'PY'
import json, sys
rows = [json.loads(l) for l in open(sys.argv[1]) if '"POST"' in l]
print(f"{len(rows)} chat requests; first healthy at +{next((r['since_first'] for r in rows if not r['down']), None)}s")
for r in rows:
    print(f"  +{r['since_first']:>6}s  {'503 no_capacity' if r['down'] else '200 SSE'}")
PY

echo "== stored notes (UI-only rows) and the assistant reply"
sqlite3 "$ROOT/forge.db" "select role, visibility, substr(content,1,160) from message where session_id in (select id from session) order by seq;" 2>&1 | head -20
exit "$RC"
