#!/usr/bin/env bash
set -euo pipefail

here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# Exit status passes through.
"$here/run-bounded.sh" 30 true
set +e
"$here/run-bounded.sh" 30 bash -c 'exit 7'
[ "$?" = 7 ] || { echo "exit status must pass through" >&2; exit 1; }

# A deadline kills a descendant that moved to its own session and exits 124.
marker="forge-bounded-$$"
RUN_BOUNDED_GRACE=1 "$here/run-bounded.sh" 2 bash -c "setsid bash -c 'exec -a $marker sleep 300' & wait"
[ "$?" = 124 ] || { echo "deadline must exit 124" >&2; exit 1; }
sleep 1
if pgrep -f "$marker" >/dev/null; then
  pkill -f "$marker" || true
  echo "setsid'd descendant survived the deadline" >&2
  exit 1
fi

# SIGTERM to the wrapper (what a cancelled job delivers) takes the tree down too.
marker="forge-bounded-term-$$"
RUN_BOUNDED_GRACE=1 "$here/run-bounded.sh" 60 bash -c "setsid bash -c 'exec -a $marker sleep 300' & wait" &
wrapper=$!
sleep 1
kill -TERM "$wrapper"
wait "$wrapper"
sleep 1
if pgrep -f "$marker" >/dev/null; then
  pkill -f "$marker" || true
  echo "descendant survived SIGTERM to the wrapper" >&2
  exit 1
fi

echo "run-bounded tree-kill behaviour verified"
