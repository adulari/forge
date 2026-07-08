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

npm ci
npx expo prebuild -p ios --no-install
cd ios

# expo prebuild always writes a fresh Info.plist with a hardcoded CFBundleVersion of "1".
# The old EAS pipeline already uploaded builds up to 11 for version 1.0.0, and Apple
# rejects any build whose CFBundleVersion isn't strictly higher than the last one
# uploaded for that version. CI_BUILD_NUMBER is Xcode Cloud's own per-product build
# counter (starts at 1) — offset it well clear of the EAS-era builds so every future
# Xcode Cloud build is guaranteed to keep increasing from here on.
/usr/libexec/PlistBuddy -c "Set :CFBundleVersion $((20 + CI_BUILD_NUMBER))" Forge/Info.plist

pod install
