#!/usr/bin/env bash
# Builds the Rust core for Android (x86_64 emulator + arm64 device) and generates Kotlin bindings.
set -euo pipefail
cd "$(dirname "$0")/.."
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/.cache/forge-target-spike-mobile}"
export ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-/opt/android-sdk/ndk/27.1.12297006}"
OUT=android/app/src/main
mkdir -p "$OUT/jniLibs" "$OUT/kotlin-gen"
cargo ndk -t x86_64 -t arm64-v8a -o "$OUT/jniLibs" build -p forge_mobile_core --release
cargo build -p forge_mobile_core
cargo run -q -p uniffi-bindgen -- generate --library "$CARGO_TARGET_DIR/debug/libforge_mobile_core.so" \
  --language kotlin --out-dir "$OUT/kotlin-gen" --config core/uniffi.toml
