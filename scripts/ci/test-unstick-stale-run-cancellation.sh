#!/usr/bin/env bash
set -euo pipefail

# Keep the stale-run sweep fail-closed: only internal PR/dispatch runs whose SHA is absent from the
# current open-PR/main set may be cancelled. This is a structural guard for the API-driven step in
# .github/workflows/unstick-automerge.yml; it does not call GitHub from ordinary CI.

workflow="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)/.github/workflows/unstick-automerge.yml"

grep -Fq 'gh api "repos/$GH_REPO/branches/main" --jq '\''.commit.sha'\''' "$workflow" \
  || { echo 'stale-run sweep must preserve the current main SHA' >&2; exit 1; }
grep -Fq 'gh pr list --state open --limit 100 --json headRefOid' "$workflow" \
  || { echo 'stale-run sweep must preserve every open PR head SHA' >&2; exit 1; }
grep -Fq 'gh pr list --state open --limit 100 --json headRefName' "$workflow" \
  || { echo 'stale-run sweep must restrict dispatch sweeps to open PR branches' >&2; exit 1; }
grep -Fq 'pull_request)' "$workflow" \
  || { echo 'stale-run sweep must not cancel tag/schedule runs' >&2; exit 1; }
grep -Fq 'workflow_dispatch)' "$workflow" \
  || { echo 'stale-run sweep must explicitly gate workflow dispatches' >&2; exit 1; }
grep -Fq 'CI|security|mobile-typecheck' "$workflow" \
  || { echo 'stale-run sweep must not cancel unrelated workflow dispatches' >&2; exit 1; }
grep -Fq '[ "$head_repo" = "$GH_REPO" ]' "$workflow" \
  || { echo 'stale-run sweep must not cancel fork runs' >&2; exit 1; }
grep -Fq 'actions/runs/$run_id/cancel' "$workflow" \
  || { echo 'stale-run sweep must call the run-level cancellation endpoint' >&2; exit 1; }
grep -Fq 'MAX_CANCELLATIONS_PER_RUN=50' "$workflow" \
  || { echo 'stale-run sweep must retain a per-run cancellation bound' >&2; exit 1; }

echo 'unstick stale-run cancellation safeguards are present'
