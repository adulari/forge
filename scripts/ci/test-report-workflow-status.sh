#!/usr/bin/env bash
# Dry run of report-workflow-status.sh against a stub `gh` that logs its calls.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
cat >"$tmp/gh" <<'STUB'
#!/usr/bin/env bash
echo "gh $*" | tr "\n" " " | sed "s/  */ /g; s/$/\n/" >>"$GH_LOG"
case "$1 $2" in
  "issue list") if [ "${OPEN_ISSUE:-}" = 1 ]; then echo "42 2020-01-01T00:00:00Z"; elif [ "${OPEN_ISSUE:-}" = fresh ]; then echo "42 $(date -u +%Y-%m-%dT%H:%M:%SZ)"; fi ;;
  "run view") echo "- e2e (step: Run the real turn)" ;;
esac
STUB
chmod +x "$tmp/gh"
run() { # name needs open expect
  export GH_LOG="$tmp/log"; : >"$GH_LOG"
  PATH="$tmp:$PATH" NEEDS_JSON="$2" OPEN_ISSUE="$3" WORKFLOW_NAME=e2e RUN_ID=7 SHA=abc REPO=o/r \
    bash "$here/report-workflow-status.sh" >/dev/null
  got=$(grep -vE '^gh (issue list|run view)' "$GH_LOG" | awk '{print $2" "$3}' | paste -sd, - || true)
  [ "$got" = "$4" ] || { echo "FAIL $1: got '$got' want '$4'"; exit 1; }
  echo "ok   $1 -> ${got:-nothing}"; cut -c1-110 "$GH_LOG" | sed "s/^/       /"
}
F='{"a":{"result":"failure"},"b":{"result":"skipped"}}'
G='{"a":{"result":"success"},"b":{"result":"skipped"}}'
C='{"a":{"result":"cancelled"},"b":{"result":"success"}}'
run "red, no issue"     "$F" 0 "issue create"
run "red, open issue"   "$F" 1 "issue comment"
run "red, issue updated just now" "$F" fresh ""
run "green, open issue" "$G" 1 "issue comment,issue close"
run "green, none"       "$G" 0 ""
run "cancelled+green"   "$C" 1 ""
