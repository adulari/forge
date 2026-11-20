#!/usr/bin/env bash
# Run a command so that neither a cancelled job nor a hung step can leave its process tree behind.
#
# WHY: a cancelled CI run left its cargo/rustc tree compiling for 90+ minutes on the one heavy
# runner, blocking every heavy job. The Actions runner's service unit uses KillMode=process, so
# only the direct child is signalled; grandchildren that are reparented or moved to another
# session keep running and keep the runner's single slot busy. This wrapper signals the whole
# DESCENDANT tree (not the process group, which a setsid'd child escapes) on INT/TERM/HUP and on
# its own deadline.
#
# usage: run-bounded.sh <timeout-seconds> <command> [args...]
set -uo pipefail

limit="${1:?usage: run-bounded.sh <timeout-seconds> <command> [args...]}"
shift
[ "$#" -gt 0 ] || { echo "run-bounded.sh: no command given" >&2; exit 2; }
grace="${RUN_BOUNDED_GRACE:-10}"

descendants() {
  local child
  for child in $(pgrep -P "$1" 2>/dev/null); do
    descendants "$child"
    echo "$child"
  done
}

kill_tree() {
  local pids
  pids="$(descendants "$$") $cmd_pid"
  # shellcheck disable=SC2086
  kill -TERM $pids 2>/dev/null
  local waited=0
  while [ "$waited" -lt "$grace" ] && kill -0 "$cmd_pid" 2>/dev/null; do
    sleep 1
    waited=$((waited + 1))
  done
  pids="$(descendants "$$") $cmd_pid"
  # shellcheck disable=SC2086
  kill -KILL $pids 2>/dev/null
}

"$@" &
cmd_pid=$!
timed_out=0
interrupted=0

trap 'interrupted=1; kill_tree' INT TERM HUP

( sleep "$limit"; kill -ALRM $$ 2>/dev/null ) &
timer_pid=$!
trap 'timed_out=1; kill_tree' ALRM

status=0
while kill -0 "$cmd_pid" 2>/dev/null; do
  wait "$cmd_pid"
  status=$?
done
kill "$timer_pid" 2>/dev/null
wait "$timer_pid" 2>/dev/null

if [ "$timed_out" = 1 ]; then
  echo "run-bounded.sh: '$1' exceeded ${limit}s and its process tree was killed" >&2
  exit 124
fi
if [ "$interrupted" = 1 ]; then
  exit 130
fi
exit "$status"
