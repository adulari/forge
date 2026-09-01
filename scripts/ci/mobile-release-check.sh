#!/usr/bin/env bash
set -euo pipefail

# The release-path form of `npm run check` for mobile/.
#
# `expo-doctor`'s "packages match versions required by installed Expo SDK" check is not hermetic:
# it resolves the SDK's *current* expected patch versions over the network, so Expo publishing a
# patch flips an already-tagged, previously-green commit red with nothing changed in the
# repository. Because app-desktop.yml checks out `refs/tags/<release_tag>`, a fix on main cannot
# rescue the tag, and the draft release never publishes. That has happened four times: #993,
# #1129, #1160, and #1213 (v2.13.4).
#
# On the release path the version comparison is therefore skipped through expo-doctor's own
# EXPO_DOCTOR_SKIP_DEPENDENCY_VERSION_CHECK knob, and network errors in the remaining checks are
# downgraded to warnings. The other 18 doctor checks, ESLint, `tsc --noEmit` and Vitest all stay
# fully enforcing — only the answer that lives outside the repository is dropped. Pull-request CI
# keeps running the plain `npm run check`, where drift is actionable by a human.
mobile_dir=${1:-"$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)/mobile"}

[ -d "$mobile_dir" ] || {
  echo "mobile directory not found: $mobile_dir" >&2
  exit 1
}

echo "::notice::release-path check: expo-doctor's SDK version comparison is skipped (non-hermetic); PR CI still enforces it"

cd "$mobile_dir"
EXPO_DOCTOR_SKIP_DEPENDENCY_VERSION_CHECK=1 \
  EXPO_DOCTOR_WARN_ON_NETWORK_ERRORS=1 \
  npm run check
