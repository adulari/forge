#!/bin/sh
set -e

# Xcode Cloud only needs a real .xcodeproj/.xcworkspace in the repo to let you
# create a workflow at all. The committed ios/ directory is just that bootstrap
# — this script regenerates it fresh from app.config.js and re-installs Pods on
# every real build, so the committed copy never has to be kept in sync by hand.

cd "$CI_PRIMARY_REPOSITORY_PATH/mobile"

# Official Node tarball, not Homebrew: Xcode 26.x workflows land on Intel hosts, where brew has no
# node@20 bottle for that macOS and its source build fails ("C compiler cannot create executables",
# builds 91/92). Homebrew also disables node@20 on 2026-10-28. The tarball works on either arch.
NODE_VERSION=20.20.2
case "$(uname -m)" in
  arm64) NODE_ARCH=arm64 ;;
  *) NODE_ARCH=x64 ;;
esac
NODE_DIST="node-v$NODE_VERSION-darwin-$NODE_ARCH"
NODE_ROOT="$HOME/.forge-node"
mkdir -p "$NODE_ROOT"
curl -fsSLo "$NODE_ROOT/$NODE_DIST.tar.gz" "https://nodejs.org/dist/v$NODE_VERSION/$NODE_DIST.tar.gz"
curl -fsSLo "$NODE_ROOT/SHASUMS256.txt" "https://nodejs.org/dist/v$NODE_VERSION/SHASUMS256.txt"
(cd "$NODE_ROOT" && grep " $NODE_DIST.tar.gz\$" SHASUMS256.txt | shasum -a 256 -c -)
tar -xzf "$NODE_ROOT/$NODE_DIST.tar.gz" -C "$NODE_ROOT"
export PATH="$NODE_ROOT/$NODE_DIST/bin:$PATH"
node -v

# --prefer-offline/--no-audit/--no-fund skip npm's registry-freshness-check and audit/funding
# network calls — pure network round-trip savings, doesn't change what gets installed.
npm ci --prefer-offline --no-audit --no-fund
npx expo prebuild -p ios --no-install

# prebuild owns ios/ and rewrites it from app.config.js every build, so a push-broken archive is a
# configuration regression rather than a code one — and one that still installs and runs fine.
# Specifically: expo-notifications' plugin writes `aps-environment: development`, EAS Build would
# rewrite that to `production` from the signing credentials, and Xcode Cloud does not. Fail here
# rather than ship a build whose notification toggle cannot be turned on.
#
# (prebuild also deletes this very ci_scripts/ directory. Harmless — Xcode Cloud clones fresh and
# runs this hook once, before the deletion — but it does mean no ci_pre_xcodebuild.sh or
# ci_post_xcodebuild.sh can be added, because prebuild would remove it before Xcode ran it.)
bash "$CI_PRIMARY_REPOSITORY_PATH/scripts/ci/verify-ios-entitlements.sh"

cd ios

# The archive's "Bundle React Native code" phase runs in a fresh xcodebuild environment without this
# PATH; .xcode.env.local is sourced after .xcode.env and pins the node it uses.
echo "export NODE_BINARY=\"$NODE_ROOT/$NODE_DIST/bin/node\"" > .xcode.env.local

# expo prebuild always regenerates project.pbxproj with CURRENT_PROJECT_VERSION = 1.
# Xcode derives the archive's real CFBundleVersion from this build setting at build
# time — it overrides whatever literal value sits in Info.plist, so patching the
# plist directly (tried first, build #5) has no effect. The old EAS pipeline already
# uploaded builds up to 11 for version 1.0.0, and Apple rejects any build whose
# CFBundleVersion isn't strictly higher than the last one uploaded for that version.
# CI_BUILD_NUMBER is Xcode Cloud's own per-product build counter (starts at 1) —
# offset it well clear of the EAS-era builds so every future build keeps increasing.
sed -i '' "s/CURRENT_PROJECT_VERSION = 1;/CURRENT_PROJECT_VERSION = $((20 + CI_BUILD_NUMBER));/g" Forge.xcodeproj/project.pbxproj

pod install
