#!/usr/bin/env bash
set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
classifier="$script_dir/changed-groups.sh"
scratch=$(mktemp -d)
trap 'rm -rf -- "$scratch"' EXIT

classify() {
  local output=$1
  shift
  GITHUB_OUTPUT="$scratch/$output" "$classifier" "$@" >/dev/null
}

expect() {
  local output=$1 group=$2 value=$3
  if ! grep -qx "$group=$value" "$scratch/$output"; then
    echo "expected $group=$value in $output" >&2
    cat "$scratch/$output" >&2
    exit 1
  fi
}

classify docs docs/README.md
for group in rust_fmt rust release_build anywhere_policy mobile_app mobile_tauri cargo_audit cargo_deny; do
  expect docs "$group" false
done

classify rust crates/forge-core/src/lib.rs
expect rust rust_fmt true
expect rust rust true
expect rust release_build true
expect rust mobile_app false
expect rust cargo_audit false

classify lock Cargo.lock
expect lock rust true
expect lock release_build true
expect lock cargo_audit true
expect lock cargo_deny true
expect lock rust_fmt false

classify manifest crates/forge-core/Cargo.toml
expect manifest rust true
expect manifest release_build true
expect manifest cargo_deny true
expect manifest cargo_audit false

classify mobile mobile/src/app/anywhere/index.tsx
expect mobile mobile_app true
expect mobile mobile_tauri false
expect mobile rust false

classify tauri mobile/src-tauri/src/serve_discovery.rs
expect tauri mobile_tauri true
expect tauri mobile_app false
expect tauri rust false

classify native mobile/ios/Forge/Info.plist
expect native mobile_app false
expect native mobile_tauri false

classify protocol protocol/remote-v9.json
expect protocol mobile_app true
expect protocol rust true
expect protocol release_build true

classify canary scripts/ci/check-anywhere-plaintext-canary.sh
expect canary anywhere_policy true
expect canary rust false

GITHUB_OUTPUT="$scratch/manual" EVENT_NAME=workflow_dispatch "$classifier" >/dev/null
for group in rust_fmt rust release_build anywhere_policy mobile_app mobile_tauri cargo_audit cargo_deny; do
  expect manual "$group" true
done

# The post-merge run on main (a `push`) scopes itself to that merge's own diff, so landing a
# docs-only change must not spend the heavy runner on the whole matrix.
repo="$scratch/push-repo"
git init -q "$repo"
git -C "$repo" -c user.email=ci@example.com -c user.name=CI commit -q --allow-empty -m base
base_sha=$(git -C "$repo" rev-parse HEAD)
mkdir -p "$repo/docs"
echo hello > "$repo/docs/README.md"
git -C "$repo" add docs/README.md
git -C "$repo" -c user.email=ci@example.com -c user.name=CI commit -q -m docs
head_sha=$(git -C "$repo" rev-parse HEAD)

(
  cd "$repo"
  GITHUB_OUTPUT="$scratch/push" EVENT_NAME=push BASE_SHA="$base_sha" HEAD_SHA="$head_sha" \
    "$classifier" >/dev/null
)
for group in rust_fmt rust release_build anywhere_policy mobile_app mobile_tauri cargo_audit cargo_deny; do
  expect push "$group" false
done

# A push with no diffable predecessor (branch creation / unresolvable force-push) must fail safe by
# running everything rather than silently checking nothing.
GITHUB_OUTPUT="$scratch/push-zero" EVENT_NAME=push \
  BASE_SHA=0000000000000000000000000000000000000000 HEAD_SHA="$head_sha" "$classifier" >/dev/null
for group in rust_fmt rust release_build anywhere_policy mobile_app mobile_tauri cargo_audit cargo_deny; do
  expect push-zero "$group" true
done

echo "Changed-file CI group classification passed"
