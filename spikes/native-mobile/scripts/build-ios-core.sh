#!/usr/bin/env bash
# macOS only. Builds the Rust core for iOS device + simulator, generates the Swift bindings, and
# wraps the static libs in an XCFramework that ios/project.yml links.
set -euo pipefail
cd "$(dirname "$0")/.."
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/.cache/forge-target-spike-mobile}"
rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
for t in aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios; do
  cargo build -p forge_mobile_core --release --target "$t"
done
cargo build -p forge_mobile_core
LIB="$CARGO_TARGET_DIR"
mkdir -p "$LIB/ios-sim-universal" ios/Generated/headers
lipo -create "$LIB/aarch64-apple-ios-sim/release/libforge_mobile_core.a" \
  "$LIB/x86_64-apple-ios/release/libforge_mobile_core.a" -output "$LIB/ios-sim-universal/libforge_mobile_core.a"
cargo run -q -p uniffi-bindgen -- generate --library "$LIB/debug/libforge_mobile_core.dylib" \
  --language swift --out-dir ios/Generated/headers
mv ios/Generated/headers/forge_mobile_core.swift ios/Generated/
# xcodebuild wants the modulemap named module.modulemap next to the header.
mv ios/Generated/headers/forge_mobile_coreFFI.modulemap ios/Generated/headers/module.modulemap
rm -rf ios/ForgeMobileCore.xcframework
xcodebuild -create-xcframework \
  -library "$LIB/aarch64-apple-ios/release/libforge_mobile_core.a" -headers ios/Generated/headers \
  -library "$LIB/ios-sim-universal/libforge_mobile_core.a" -headers ios/Generated/headers \
  -output ios/ForgeMobileCore.xcframework
