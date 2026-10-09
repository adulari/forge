#!/usr/bin/env bash
# Point Cargo at a build directory that survives between jobs on this persistent runner.
#
# WHY: actions/checkout runs `git clean -ffdx` before every job, which deletes the workspace's
# `target/` and `vendor/genai-0.6.5/target/`. The old "target/ survives on local disk" assumption
# was therefore false: every clippy/test/release-build job recompiled ~770 crates cold (3m35s of a
# 6m test job). A directory beside the checkout, under the runner's work root, is not touched by
# the clean.
#
# The leaf directory MUST be named `target`: the secret-store tripwire in forge-config recognises a
# cargo-built test binary by a `target/**/deps/` path component and only then swaps in the
# in-memory store. Any other name would let tests reach the real keyring.
#
# Usage: cargo-cache-env.sh <slot> [<vendor-slot>]   (slots keep profiles that would evict each
# other, e.g. release vs dev, in separate trees)
set -euo pipefail

slot="${1:?usage: cargo-cache-env.sh <slot> [<vendor-slot>]}"
vendor_slot="${2:-}"
case "$slot$vendor_slot" in
  *[!A-Za-z0-9_-]*)
    echo "cache slot names must be [A-Za-z0-9_-]" >&2
    exit 2
    ;;
esac

work_root="${RUNNER_WORKSPACE:-}"
if [[ -z "$work_root" ]]; then
  echo "RUNNER_WORKSPACE is not set; leaving Cargo's target directory alone" >&2
  exit 0
fi

cache_root="$work_root/.cargo-ci"
mkdir -p "$cache_root/$slot/target"

out="${GITHUB_ENV:-/dev/stdout}"
{
  echo "CARGO_TARGET_DIR=$cache_root/$slot/target"
  echo "FORGE_CARGO_CACHE_ROOT=$cache_root"
  if [[ -n "$vendor_slot" ]]; then
    mkdir -p "$cache_root/$vendor_slot/target"
    echo "FORGE_VENDOR_TARGET_DIR=$cache_root/$vendor_slot/target"
  fi
} >> "$out"
