#!/usr/bin/env bash
set -euo pipefail

# Real-display operator harness for the retained app. This intentionally does not
# guess DOM coordinates: it records compositor placement, then leaves the
# operator to open /perf-fixture and Diagnostics using the visible app. The
# diagnostics surface is the source of interactive, input, frame, and long-task
# values; this script only automates launch/cleanup and process RSS sampling.

binary=${1:?usage: $0 /path/to/forge-desktop [cold|warm] [seconds]}
label=${2:-capture}
duration=${3:-30}
log=/tmp/forge-performance-$$.log
start_ns=$(date +%s%N)
"$binary" >"$log" 2>&1 &
root_pid=$!
cleanup() { kill "$root_pid" 2>/dev/null || true; wait "$root_pid" 2>/dev/null || true; }
trap cleanup EXIT INT TERM

client_pid=""
for _ in $(seq 1 300); do
  client_pid=$(hyprctl clients -j | python3 -c 'import json,sys; root=int("'"$root_pid"'"); print(next((str(x["pid"]) for x in json.load(sys.stdin) if x["pid"] == root), ""))')
  [[ -n "$client_pid" ]] && break
  sleep 0.1
done
[[ -n "$client_pid" ]] || { echo "window did not map; see $log" >&2; exit 1; }
mapped_ns=$(date +%s%N)

printf 'capture=%s\n' "$label"
printf 'mapped_ms=%.3f\n' "$((mapped_ns-start_ns))e-6"
hyprctl monitors -j | python3 -c 'import json,sys; print(json.dumps([{k:m[k] for k in ("name","width","height","refreshRate","scale","focused")} for m in json.load(sys.stdin)]))'
hyprctl clients -j | python3 -c 'import json,sys; root=int("'"$root_pid"'"); print(json.dumps([{"pid":x["pid"],"title":x["title"],"monitor":x["monitor"],"at":x["at"],"size":x["size"]} for x in json.load(sys.stdin) if x["pid"] == root]))'
echo "Open /perf-fixture in the debug build and run the documented scroll/stream procedure. Read values from Diagnostics; no input is synthesized by this script."

sleep "$duration"
ps -o pid,ppid,rss,vsz,stat,comm -p "$(pgrep -P "$root_pid" | paste -sd, -),$root_pid" 2>/dev/null || true
