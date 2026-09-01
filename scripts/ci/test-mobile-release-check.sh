#!/usr/bin/env bash
set -euo pipefail

# Guards scripts/ci/mobile-release-check.sh. The point of the script is that a tagged release
# survives an upstream Expo patch publish, so the drifted-lockfile case is tested first.

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd -- "$script_dir/../.." && pwd)
check="$script_dir/mobile-release-check.sh"
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT

# The fixtures run real `npm run` scripts; a missing toolchain must be an explicit failure rather
# than a confusing one from inside a case.
command -v npm >/dev/null || { echo 'npm is required to test the release-path check' >&2; exit 1; }

# A stand-in for mobile/ whose `check` script fails exactly like expo-doctor's non-hermetic
# dependency-version check: green only when the release path has disabled it.
fake_mobile() {
  local dir="$1"
  mkdir -p "$dir"
  cat > "$dir/package.json" <<'JSON'
{
  "name": "fake-mobile",
  "version": "1.0.0",
  "private": true,
  "scripts": {
    "check": "node -e \"if (!process.env.EXPO_DOCTOR_SKIP_DEPENDENCY_VERSION_CHECK) { console.error('expo 57.0.18 != ~57.0.19'); process.exit(1); } console.log('19/19 checks passed')\""
  }
}
JSON
}

# 1. One Expo patch behind: the release path must still pass.
fake_mobile "$work/behind"
bash "$check" "$work/behind" >/dev/null 2>&1 \
  || { echo 'a lockfile one Expo patch behind must not fail the release path' >&2; exit 1; }

# 2. The same tree must fail a plain `npm run check`, or case 1 proves nothing.
if (cd "$work/behind" && npm run --silent check >/dev/null 2>&1); then
  echo 'the fixture must fail the enforcing PR-CI check' >&2
  exit 1
fi

# 3. Real failures still fail: lint, tsc and Vitest stay enforcing on the release path.
mkdir -p "$work/broken"
cat > "$work/broken/package.json" <<'JSON'
{
  "name": "fake-mobile-broken",
  "version": "1.0.0",
  "private": true,
  "scripts": { "check": "node -e \"process.exit(3)\"" }
}
JSON
if bash "$check" "$work/broken" >/dev/null 2>&1; then
  echo 'a failing lint/tsc/test run must fail the release path' >&2
  exit 1
fi

# 4. A missing project directory is an error, not a silent pass.
if bash "$check" "$work/absent" >/dev/null 2>&1; then
  echo 'a missing mobile directory must fail' >&2
  exit 1
fi

# 5. Against the real project: expo-doctor must report all checks passing with the version
#    comparison disabled even when installed packages are a patch behind. Needs an install.
if [ -d "$repo_root/mobile/node_modules/expo-doctor" ]; then
  drifted="$work/drifted"
  mkdir -p "$drifted"
  # A tracked-files copy, so the config resolves exactly as it does in CI.
  (cd "$repo_root" && git ls-files -z mobile) \
    | (cd "$repo_root" && xargs -0 tar -cf -) \
    | (cd "$drifted" && tar -xf - --strip-components=1)
  ln -s "$repo_root/mobile/node_modules" "$drifted/node_modules"
  # Roll the SDK package one patch back, exactly the drift an upstream publish creates.
  node -e '
    const fs = require("fs");
    const pkg = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
    const cur = pkg.dependencies.expo.replace(/[^0-9.]/g, "").split(".");
    cur[2] = String(Math.max(0, Number(cur[2]) - 1));
    pkg.dependencies.expo = "~" + cur.join(".");
    fs.writeFileSync(process.argv[2], JSON.stringify(pkg, null, 2));
  ' "$repo_root/mobile/package.json" "$drifted/package.json"
  if ! output=$(cd "$drifted" && EXPO_DOCTOR_SKIP_DEPENDENCY_VERSION_CHECK=1 \
    EXPO_DOCTOR_WARN_ON_NETWORK_ERRORS=1 npx --no-install expo-doctor 2>&1); then
    echo 'expo-doctor must pass on a drifted project when the version check is skipped' >&2
    echo "$output" >&2
    exit 1
  fi
  grep -q 'checks passed' <<<"$output" || {
    echo 'expected a passing expo-doctor summary' >&2
    echo "$output" >&2
    exit 1
  }
else
  echo 'note: mobile/node_modules absent, skipping the live expo-doctor case'
fi

echo 'mobile-release-check.sh: ok'
