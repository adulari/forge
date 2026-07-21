#!/usr/bin/env bash
set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
classifier="$script_dir/eas-ota-safety.sh"
scratch=$(mktemp -d)
trap 'rm -rf -- "$scratch"' EXIT
failures=0

classify() {
  local output=$1
  shift
  GITHUB_OUTPUT="$scratch/$output" "$classifier" "$@" >/dev/null
}

expect() {
  local output=$1 key=$2 value=$3
  if ! grep -qx "$key=$value" "$scratch/$output"; then
    echo "expected $key=$value in $output" >&2
    cat "$scratch/$output" >&2
    failures=$((failures + 1))
  fi
}

fail() {
  echo "$*" >&2
  failures=$((failures + 1))
}

classify source_and_docs mobile/src/app.tsx docs/plan.md
expect source_and_docs safe true
expect source_and_docs ota_changed true

classify assets_and_rust mobile/assets/icon.png crates/core/src/lib.rs
expect assets_and_rust safe true
expect assets_and_rust ota_changed true

classify docs_only docs/plan.md
expect docs_only safe true
expect docs_only ota_changed false

unsafe_paths=(
  mobile/ios/Info.plist
  mobile/android/app/build.gradle
  mobile/plugins/with-native-module.js
  mobile/app.json
  mobile/app.config.ts
  mobile/eas.json
  mobile/package.json
  mobile/package-lock.json
  mobile/PrivacyInfo.xcprivacy
  mobile/metro.config.js
  mobile/babel.config.js
  mobile/unknown-build-config.js
)
for index in "${!unsafe_paths[@]}"; do
  output="unsafe_$index"
  classify "$output" mobile/src/app.tsx "${unsafe_paths[$index]}"
  expect "$output" safe false
  expect "$output" ota_changed true
done

neutral_paths=(
  docs/plan.md
  crates/core/src/lib.rs
  .github/workflows/ci.yml
  Cargo.toml
  scripts/ci/helper.sh
  mobile/src-tauri/src/lib.rs
  mobile/redesign/mockup.png
  mobile/README.md
)
for index in "${!neutral_paths[@]}"; do
  output="neutral_$index"
  classify "$output" mobile/assets/icon.png "${neutral_paths[$index]}"
  expect "$output" safe true
  expect "$output" ota_changed true
done

GITHUB_OUTPUT="$scratch/manual" EVENT_NAME=workflow_dispatch "$classifier" >/dev/null
expect manual safe true
expect manual ota_changed true

rename_repo="$scratch/rename-repo"
git init -q "$rename_repo"
git -C "$rename_repo" config user.name "EAS OTA safety test"
git -C "$rename_repo" config user.email "eas-ota-safety@example.invalid"
mkdir -p "$rename_repo/docs" "$rename_repo/mobile/src"
printf 'lock\n' > "$rename_repo/mobile/package-lock.json"
git -C "$rename_repo" add mobile/package-lock.json
git -C "$rename_repo" commit -q -m "unsafe dependency"
rename_base=$(git -C "$rename_repo" rev-parse HEAD)
git -C "$rename_repo" mv mobile/package-lock.json docs/old-lock.json
printf 'app\n' > "$rename_repo/mobile/src/app.tsx"
git -C "$rename_repo" add mobile/src/app.tsx
git -C "$rename_repo" commit -q -m "rename dependency and update source"
rename_head=$(git -C "$rename_repo" rev-parse HEAD)
if ! git -C "$rename_repo" diff --name-status "$rename_base" "$rename_head" | grep -q '^R100'; then
  fail "rename regression setup did not produce a detected rename"
fi
(
  cd "$rename_repo"
  GITHUB_OUTPUT="$scratch/unsafe_rename" \
    EVENT_NAME=push \
    BASE_SHA="$rename_base" \
    HEAD_SHA="$rename_head" \
    "$classifier" >/dev/null
)
expect unsafe_rename safe false
expect unsafe_rename ota_changed true

range_repo="$scratch/range-repo"
git init -q "$range_repo"
git -C "$range_repo" config user.name "EAS OTA safety test"
git -C "$range_repo" config user.email "eas-ota-safety@example.invalid"
mkdir -p "$range_repo/docs" "$range_repo/mobile/src"
printf 'plan\n' > "$range_repo/docs/plan.md"
git -C "$range_repo" add docs/plan.md
git -C "$range_repo" commit -q -m "docs"
printf '{}\n' > "$range_repo/mobile/package.json"
git -C "$range_repo" add mobile/package.json
git -C "$range_repo" commit -q -m "unsafe dependency"
printf 'app\n' > "$range_repo/mobile/src/app.tsx"
git -C "$range_repo" add mobile/src/app.tsx
git -C "$range_repo" commit -q -m "mobile source"
range_head=$(git -C "$range_repo" rev-parse HEAD)

for base_case in all_zero empty; do
  if [[ $base_case == all_zero ]]; then
    base_sha=0000000000000000000000000000000000000000
  else
    base_sha=
  fi
  (
    cd "$range_repo"
    GITHUB_OUTPUT="$scratch/$base_case" \
      EVENT_NAME=push \
      BASE_SHA="$base_sha" \
      HEAD_SHA="$range_head" \
      "$classifier" >/dev/null
  )
  expect "$base_case" safe false
  expect "$base_case" ota_changed false
done

invalid_output="$scratch/invalid_range"
if (
  cd "$range_repo"
  GITHUB_OUTPUT="$invalid_output" \
    EVENT_NAME=push \
    BASE_SHA=definitely-not-a-sha \
    HEAD_SHA="$range_head" \
    "$classifier" >/dev/null 2>&1
); then
  fail "invalid git range unexpectedly exited zero"
fi
if [[ -s $invalid_output ]]; then
  fail "invalid git range unexpectedly emitted outputs"
fi

nul_repo="$scratch/nul-repo"
git init -q "$nul_repo"
git -C "$nul_repo" config user.name "EAS OTA safety test"
git -C "$nul_repo" config user.email "eas-ota-safety@example.invalid"
mkdir -p "$nul_repo/docs" "$nul_repo/mobile/src"
printf 'plan\n' > "$nul_repo/docs/plan.md"
git -C "$nul_repo" add docs/plan.md
git -C "$nul_repo" commit -q -m "docs"
nul_base=$(git -C "$nul_repo" rev-parse HEAD)
odd_unsafe_path=$'mobile/unknown\nconfig.js'
printf 'config\n' > "$nul_repo/$odd_unsafe_path"
printf 'app\n' > "$nul_repo/mobile/src/app.tsx"
git -C "$nul_repo" add -- "$odd_unsafe_path" mobile/src/app.tsx
git -C "$nul_repo" commit -q -m "odd unsafe path and source"
nul_head=$(git -C "$nul_repo" rev-parse HEAD)
(
  cd "$nul_repo"
  GITHUB_OUTPUT="$scratch/nul_path" \
    EVENT_NAME=push \
    BASE_SHA="$nul_base" \
    HEAD_SHA="$nul_head" \
    "$classifier" >/dev/null
)
expect nul_path safe false
expect nul_path ota_changed true

if ((failures)); then
  echo "EAS OTA safety classification failed with $failures assertion(s)" >&2
  exit 1
fi

echo "EAS OTA safety classification passed"
