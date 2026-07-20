#!/usr/bin/env bash
set -euo pipefail

# Classify a pull request's changed files into the smallest useful CI groups.
# Scheduled and manually dispatched workflows intentionally run every group.
#
# For local verification, pass paths as arguments instead of providing a git
# range. GitHub Actions writes the resulting booleans to GITHUB_OUTPUT.

groups=(
  rust_fmt
  rust
  release_build
  anywhere_policy
  mobile_app
  mobile_tauri
  cargo_audit
  cargo_deny
)

declare -A enabled=()
for group in "${groups[@]}"; do
  enabled["$group"]=false
done

enable() {
  local group
  for group in "$@"; do
    enabled["$group"]=true
  done
}

enable_all() {
  enable "${groups[@]}"
}

classify() {
  local path=$1

  case "$path" in
    scripts/ci/changed-groups.sh|scripts/ci/test-changed-groups.sh)
      # A classifier change must exercise every conditional job once.
      enable_all
      ;;
    .github/workflows/ci.yml)
      enable rust_fmt rust release_build anywhere_policy
      ;;
    .github/workflows/mobile-typecheck.yml)
      enable mobile_app mobile_tauri
      ;;
    .github/workflows/security.yml)
      enable cargo_audit cargo_deny
      ;;
    scripts/ci/check-anywhere-plaintext-canary.sh|scripts/ci/test-anywhere-plaintext-canary.sh)
      enable anywhere_policy
      ;;
    scripts/release-upgrade-e2e.sh)
      enable release_build
      ;;
    rustfmt.toml|vendor/*/rustfmt.toml)
      enable rust_fmt
      ;;
    rust-toolchain.toml|rust-toolchain)
      enable rust_fmt rust release_build
      ;;
    Cargo.lock|vendor/*/Cargo.lock)
      enable rust release_build cargo_audit cargo_deny
      ;;
    Cargo.toml|crates/*/Cargo.toml|vendor/*/Cargo.toml|.cargo/*)
      enable rust release_build cargo_deny
      ;;
    deny.toml)
      enable cargo_deny
      ;;
    protocol/remote-v8.json)
      enable rust release_build mobile_app
      ;;
    crates/*|vendor/*)
      enable rust release_build
      if [[ "$path" == *.rs ]]; then
        enable rust_fmt
      fi
      ;;
    mobile/src-tauri/*)
      enable mobile_tauri
      ;;
    mobile/ios/*|mobile/android/*|mobile/redesign/*|mobile/*.md|mobile/PrivacyInfo.xcprivacy)
      # Native release pipelines are explicit/manual. These files do not affect
      # the JavaScript or Tauri checks in mobile-typecheck.
      :
      ;;
    mobile/*|protocol/*)
      enable mobile_app
      ;;
  esac
}

event_name=${EVENT_NAME:-${GITHUB_EVENT_NAME:-}}

if (($#)); then
  for path in "$@"; do
    classify "$path"
  done
elif [[ "$event_name" != pull_request ]]; then
  enable_all
else
  base_sha=${BASE_SHA:?BASE_SHA is required for pull_request classification}
  head_sha=${HEAD_SHA:?HEAD_SHA is required for pull_request classification}
  while IFS= read -r -d '' path; do
    classify "$path"
  # Deletions matter too: removing a source, manifest, or lockfile must run the
  # same checks as adding or editing it.
  done < <(git diff --name-only -z "$base_sha" "$head_sha")
fi

output_file=${GITHUB_OUTPUT:-/dev/stdout}
for group in "${groups[@]}"; do
  printf '%s=%s\n' "$group" "${enabled[$group]}" >> "$output_file"
done

printf 'CI groups:'
for group in "${groups[@]}"; do
  if [[ ${enabled[$group]} == true ]]; then
    printf ' %s' "$group"
  fi
done
printf '\n'
