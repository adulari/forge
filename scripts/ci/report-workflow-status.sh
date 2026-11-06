#!/usr/bin/env bash
# Keeps ONE tracking issue per workflow in step with that workflow's health: opens (or comments on)
# it when a run is red, closes it with a link to the first green run. Called from the final
# `report` job of the scheduled/push/release workflows, never from pull_request CI.
#
# Inputs (env):
#   NEEDS_JSON     toJSON(needs) of the calling job. Verdict comes from each job's `result`:
#                  any `failure` = red; `skipped` is neutral; `cancelled` is not a failure, and
#                  blocks closing (a cancelled leg proved nothing).
#   WORKFLOW_NAME  github.workflow       RUN_ID  github.run_id       SHA  github.sha
#   REPO           github.repository     SERVER_URL  github.server_url
#   GH_TOKEN       needs issues: write and actions: read
#
# The issue is found by a hidden marker in its body, not by title alone, so a human editing the
# title does not orphan it. Same one-issue-commented-repeatedly policy as flake-hunt.yml.
set -euo pipefail

: "${NEEDS_JSON:?}" "${WORKFLOW_NAME:?}" "${RUN_ID:?}" "${SHA:?}" "${REPO:?}"
SERVER_URL=${SERVER_URL:-https://github.com}
RUN_URL="$SERVER_URL/$REPO/actions/runs/$RUN_ID"
export MARKER="<!-- ci-watch:$WORKFLOW_NAME -->"
TITLE="CI red: $WORKFLOW_NAME"

failed=$(jq '[.[] | select(.result == "failure")] | length' <<<"$NEEDS_JSON")
cancelled=$(jq '[.[] | select(.result == "cancelled")] | length' <<<"$NEEDS_JSON")
succeeded=$(jq '[.[] | select(.result == "success")] | length' <<<"$NEEDS_JSON")

if [ "$failed" -gt 0 ]; then
  verdict=red
elif [ "$cancelled" -eq 0 ] && [ "$succeeded" -gt 0 ]; then
  verdict=green
else
  echo "verdict neutral (failed=$failed cancelled=$cancelled succeeded=$succeeded): nothing to do"
  exit 0
fi

number=$(gh issue list --repo "$REPO" --state open --limit 200 --json number,body \
  --jq '[.[] | select(.body | contains(env.MARKER))] | .[0].number // empty' )

if [ "$verdict" = green ]; then
  if [ -z "$number" ]; then
    echo "green, no open issue: nothing to do"
    exit 0
  fi
  gh issue comment "$number" --repo "$REPO" \
    --body "Green again: $RUN_URL (commit \`$SHA\`). Closing; a new red run reopens tracking in a fresh issue."
  gh issue close "$number" --repo "$REPO" --reason completed
  exit 0
fi

# Best effort: the issue is still worth filing if the jobs API is unavailable.
detail=$(gh run view "$RUN_ID" --repo "$REPO" --json jobs \
  --jq '.jobs[] | select(.conclusion == "failure") |
        "- " + .name + (
          [.steps[]? | select(.conclusion == "failure") | .name] as $s
          | if ($s | length) > 0 then " (step: " + ($s | join(", ")) + ")" else "" end)' \
  2>/dev/null || true)
[ -n "$detail" ] || detail="- (failing jobs unavailable; see the run)"

body=$(mktemp)
{
  echo "$MARKER"
  echo
  echo "Workflow \`$WORKFLOW_NAME\` failed."
  echo
  echo "Run: $RUN_URL"
  echo "Commit: \`$SHA\`"
  echo
  echo "Failing jobs:"
  echo "$detail"
  echo
  echo "This issue is opened by the \`report\` job of the workflow and closed automatically by its next green run."
} >"$body"

if [ -n "$number" ]; then
  gh issue comment "$number" --repo "$REPO" --body-file "$body"
else
  gh issue create --repo "$REPO" --title "$TITLE" --body-file "$body"
fi
