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
pod install
