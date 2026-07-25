#!/bin/sh
set -e

# Xcode Cloud only needs a real .xcodeproj/.xcworkspace in the repo to let you
# create a workflow at all. The committed ios/ directory is just that bootstrap
# — this script regenerates it fresh from app.config.js and re-installs Pods on
# every real build, so the committed copy never has to be kept in sync by hand.

cd "$CI_PRIMARY_REPOSITORY_PATH/mobile"

brew install node@20
brew link --overwrite --force node@20
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
