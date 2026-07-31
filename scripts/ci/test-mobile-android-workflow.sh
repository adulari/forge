#!/usr/bin/env bash
set -euo pipefail

workflow=".github/workflows/mobile-android.yml"
config="mobile/eas.json"
lockfile="mobile/package-lock.json"

grep -Fq -- '--profile "$PROFILE"' "$workflow" || {
  echo 'Android workflow must select the requested EAS build profile' >&2
  exit 1
}
if grep -Eq -- '(^|[[:space:]])--environment([=[:space:]]|$)' "$workflow"; then
  echo 'eas build does not accept --environment; configure it in eas.json' >&2
  exit 1
fi
grep -Fq "sed -n '1,200p' build.json >&2" "$workflow" || {
  echo 'Android workflow must expose bounded EAS failure output' >&2
  exit 1
}

node - "$config" "$lockfile" <<'NODE'
const fs = require("node:fs");
const config = JSON.parse(fs.readFileSync(process.argv[2], "utf8"));
const lockfile = JSON.parse(fs.readFileSync(process.argv[3], "utf8"));
const installedCli = lockfile.packages?.["node_modules/eas-cli"]?.version;
if (!installedCli || config.cli?.version !== installedCli) {
  throw new Error("eas.json must pin the EAS CLI version resolved in package-lock.json");
}
const expected = {
  preview: "apk",
  production: "app-bundle",
};
for (const [profile, buildType] of Object.entries(expected)) {
  const build = config.build?.[profile];
  if (build?.environment !== profile) {
    throw new Error(`${profile} must select the matching EAS environment`);
  }
  if (build?.android?.buildType !== buildType) {
    throw new Error(`${profile} Android buildType must remain ${buildType}`);
  }
}
NODE

echo "Android EAS build workflow contract passed"
