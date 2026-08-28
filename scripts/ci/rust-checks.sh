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
cargo clippy --locked --manifest-path vendor/genai-0.6.5/Cargo.toml --all-targets -- -D warnings
