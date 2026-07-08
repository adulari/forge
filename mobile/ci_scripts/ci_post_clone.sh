#!/bin/sh
# Xcode Cloud runs this right after cloning the repo, before opening the Xcode project. This repo
# has no checked-in ios/ folder (Expo CNG) — prebuild materializes it fresh on every run, the same
# way `eas build` already does. --no-install skips a redundant `pod install` re-run; CocoaPods
# still runs normally as part of the Xcode Cloud build step afterward.
set -e

cd "$(dirname "$0")/.."

npm ci
npx expo prebuild --platform ios --no-install
