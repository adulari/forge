#!/usr/bin/env bash
set -uo pipefail

# Run the test suite repeatedly and report every test that fails at least once.
#
# CI runs each suite ONCE, so a race that fails one run in ten shows up as a red PR, goes green on
# a re-run, and is never diagnosed. Four such races were found in 2026-08-07..10 (#1003, #1005,
# #1009, #1011) purely by running suites in a loop by hand; chasing one of them also turned up
# #1004, a genuine production defect. This script is that method, written down.
#
# Two behaviours matter more than the loop itself:
#   1. KEEP THE FIRST FAILING LOG. Re-running and reporting green is how the cause gets lost.
#      Every one of those four was only diagnosable because the failing output was captured.
#   2. REPORT PER-TEST COUNTS. "4/30" tells you it is a race worth chasing; "it failed once" does
#      not, and gets dismissed.
#
# Usage:
#   scripts/flake-hunt.sh [runs] [cargo test args...]
#   RUNS=50 scripts/flake-hunt.sh                      # whole workspace, 50 times
#   scripts/flake-hunt.sh 30 -p forge-agent --bin forge
#
# Deliberately NOT wired into per-PR CI: the `heavy` label resolves to a single runner and the
# merge queue is already the bottleneck. Run it on a schedule or by hand.

runs="${RUNS:-20}"
if [[ "${1:-}" =~ ^[0-9]+$ ]]; then
  runs="$1"
  shift
fi

out="${FLAKE_OUT:-$(mktemp -d)}"
mkdir -p "$out"
failures=0

echo "flake hunt: $runs runs of \`cargo test ${*:-（workspace)}\`"
echo "logs: $out"
echo

for i in $(seq 1 "$runs"); do
  log="$out/run-$i.log"
  # A disposable store per run: several tests reach the default store when FORGE_DB is unset, and
  # a bare run would migrate the developer's real one (see AGENTS.md).
  FORGE_DB="$(mktemp -d)/forge.db" cargo test "$@" >"$log" 2>&1
  code=$?
  if [ "$code" -eq 0 ]; then
    rm -f "$log"
    printf '.'
    continue
  fi
  failures=$((failures + 1))
  printf 'F'
  # Record which tests failed this run. `cargo test` lists them under a `failures:` block.
  awk '/^failures:$/{f=1;next} /^$/{f=0} f&&/^    /{print $1}' "$log" | sort -u >>"$out/failed-tests.txt"
done

echo
echo

if [ "$failures" -eq 0 ]; then
  echo "no failures in $runs runs"
  exit 0
fi

echo "$failures of $runs runs failed"
echo
echo "per-test failure counts:"
sort "$out/failed-tests.txt" | uniq -c | sort -rn | sed 's/^/  /'
echo
echo "first failing log kept at: $(ls "$out"/run-*.log 2>/dev/null | head -1)"
echo
echo "A test failing a fraction of runs is a race, not noise — diagnose it from the kept log"
echo "rather than re-running until it passes."
exit 1
