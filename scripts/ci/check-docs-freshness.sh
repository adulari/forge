#!/usr/bin/env bash
set -euo pipefail

# Keep a small set of standing docs beside the source surfaces whose user-visible contract they
# describe. This is deliberately a changed-path check, not a prose linter: code-only changes in
# unrelated areas stay cheap, while a change to one of these contracts must refresh its paragraph.

declare -a pairs=(
  "docs/features/lattice-token-savings.md|crates/forge-index/src/"
  "docs/features/remote-control.md|crates/forge-cli/src/serve_terminal.rs"
  "mobile/EAS_UPDATE.md|.github/workflows/eas-update.yml .github/workflows/mobile-ota-reconcile.yml"
  ".github/workflows/flake-hunt.yml|scripts/flake-hunt.sh"
)

if (($#)); then
  changed=("$@")
elif [[ "${EVENT_NAME:-${GITHUB_EVENT_NAME:-}}" == pull_request ]]; then
  base=${BASE_SHA:?BASE_SHA is required for pull_request freshness checks}
  head=${HEAD_SHA:?HEAD_SHA is required for pull_request freshness checks}
  mapfile -d '' changed < <(git diff --name-only -z "$base" "$head")
else
  # Scheduled/manual CI has no review diff to compare; the next PR touching a mapped source will
  # enforce the pairing. This keeps the check from inventing a stale baseline on a full run.
  exit 0
fi

has_path() {
  local candidate=$1 path
  for path in "${changed[@]}"; do
    [[ "$path" == "$candidate" ]] && return 0
  done
  return 1
}

has_source() {
  local patterns=$1 pattern path
  read -r -a patterns_array <<< "$patterns"
  for pattern in "${patterns_array[@]}"; do
    for path in "${changed[@]}"; do
      case "$path" in
        "$pattern"|"$pattern"*) return 0 ;;
      esac
    done
  done
  return 1
}

failed=0
for pair in "${pairs[@]}"; do
  doc=${pair%%|*}
  sources=${pair#*|}
  if has_source "$sources" && ! has_path "$doc"; then
    echo "::error file=$doc::standing documentation is stale for changed source ($sources); update $doc in the same PR"
    failed=1
  fi
done

if ((failed)); then
  exit 1
fi
echo "standing docs freshness check passed"
