#!/usr/bin/env bash
# Single source of truth for the workspace clippy check.
#
# WHY: .github/workflows/ci.yml sets `RUSTFLAGS: -D warnings` at the workflow level, so CI's
# `cargo clippy` promotes dead_code and unused_imports to hard errors while a developer running
# the same command locally sees only warnings. Two branches passed locally and failed CI on
# exactly that drift. CI and local runs now invoke this script, so the flag lives in one place.
set -euo pipefail

export RUSTFLAGS="${RUSTFLAGS:--D warnings}"

cargo clippy --locked --all-targets --all-features
# The vendored genai workspace has its own lockfile, so it gets its own target tree in CI rather
# than fighting the main one over the same fingerprints.
CARGO_TARGET_DIR="${FORGE_VENDOR_TARGET_DIR:-${CARGO_TARGET_DIR:-$PWD/vendor/genai-0.6.5/target}}" \
  cargo clippy --locked --manifest-path vendor/genai-0.6.5/Cargo.toml --all-targets -- -D warnings
