#!/usr/bin/env bash
set -euo pipefail

# Display-independent capture helper. It never changes compositor configuration.
# Launch the binary supplied as $1, wait for its Hyprland client, and sample the
# complete descendant process tree. Use the diagnostics surface for interactive
# and frame/long-task measurements; this helper only measures launch visibility
# and RSS because it cannot establish app interactivity.

binary=${1:?usage: $0 /path/to/forge-desktop [cold|warm] [seconds]}
label=${2:-capture}
duration=${3:-0}
start_ns=$(date +%s%N)
"$binary" >/tmp/forge-performance-"$$".log 2>&1 &
root_pid=$!
cleanup() {
  kill "$root_pid" 2>/dev/null || true
  wait "$root_pid" 2>/dev/null || true
}
trap cleanup EXIT

client_pid=
for _ in $(seq 1 300); do
  client_pid=$(hyprctl clients -j | python3 -c 'import json,sys; root=int("'"$root_pid"'"); print(next((str(x["pid"]) for x in json.load(sys.stdin) if x["pid"] == root), ""))')
  if [[ -n "$client_pid" ]]; then break; fi
  sleep 0.1
done
if [[ -z "$client_pid" ]]; then
  echo "window did not map; root_pid=$root_pid" >&2
  exit 1
fi
mapped_ns=$(date +%s%N)

hyprctl monitors -j | python3 -c 'import json,sys; print(json.dumps([{k:m[k] for k in ("name","width","height","refreshRate","scale","focused")} for m in json.load(sys.stdin)]))'
hyprctl clients -j | python3 -c 'import json,sys; print(json.dumps([{"pid":x["pid"],"class":x["class"],"title":x["title"],"monitor":x["monitor"],"at":x["at"],"size":x["size"]} for x in json.load(sys.stdin) if x["pid"] == int("'"$client_pid"'")]))'
printf 'label=%s root_pid=%s client_pid=%s mapped_ms=%.3f\n' "$label" "$root_pid" "$client_pid" "$((mapped_ns-start_ns))e-6"

# Include the root and all descendants. RSS is reported in KiB by ps.
mapfile -t pids < <(python3 - "$root_pid" <<'PY'
import pathlib, sys
root = int(sys.argv[1])
children = {}
for status in pathlib.Path("/proc").glob("[0-9]*/status"):
    try:
        fields = dict(line.split(":", 1) for line in status.read_text().splitlines() if ":" in line)
        children.setdefault(int(fields["PPid"].strip()), []).append(int(fields["Pid"].strip()))
    except (FileNotFoundError, KeyError, ValueError):
        pass
seen, todo = set(), [root]
while todo:
    pid = todo.pop()
    if pid in seen: continue
    seen.add(pid)
    todo.extend(children.get(pid, []))
print("\n".join(map(str, sorted(seen))))
PY
)
ps -o pid,ppid,rss,vsz,stat,comm -p "$(IFS=,; echo "${pids[*]}")"
if [[ "$duration" != 0 ]]; then
  end=$((SECONDS + duration))
  peak=0
  last=0
  while (( SECONDS < end )); do
    mapfile -t rss_values < <(ps -o rss= -p "$(IFS=,; echo "${pids[*]}")" | awk '{ total += $1 } END { print total + 0 }')
    last=${rss_values[0]:-0}
    (( last > peak )) && peak=$last
    sleep 0.5
  done
  printf 'sample_duration_s=%s idle_or_end_tree_rss_kib=%s peak_tree_rss_kib=%s\n' "$duration" "$last" "$peak"
fi
