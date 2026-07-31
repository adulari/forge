#!/usr/bin/env bash
set -euo pipefail

workflow=".github/workflows/mobile-android.yml"
ota_workflow=".github/workflows/eas-update.yml"
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

node - "$config" "$lockfile" "$workflow" "$ota_workflow" <<'NODE'
const fs = require("node:fs");
const config = JSON.parse(fs.readFileSync(process.argv[2], "utf8"));
const lockfile = JSON.parse(fs.readFileSync(process.argv[3], "utf8"));
const workflow = fs.readFileSync(process.argv[4], "utf8");
const otaWorkflow = fs.readFileSync(process.argv[5], "utf8");
const cliVersion = config.cli?.version;
if (!/^\d+\.\d+\.\d+$/.test(cliVersion ?? "")) {
  throw new Error("eas.json must pin an exact EAS CLI version");
}
if (lockfile.packages?.["node_modules/eas-cli"]) {
  throw new Error("eas-cli must stay out of project dependencies; invoke the pinned CLI with npx");
}
const cliCommand = `npx --yes eas-cli@${cliVersion}`;
for (const command of ["build", "submit"]) {
  if (!workflow.includes(`${cliCommand} ${command}`)) {
    throw new Error(`Android workflow must invoke pinned EAS CLI for ${command}`);
  }
}
if (!otaWorkflow.includes(`${cliCommand} update`)) {
  throw new Error("OTA workflow must invoke the same pinned EAS CLI");
}

const configuredNode = config.build?.base?.node;
const parseVersion = (version) => {
  const match = /^(\d+)\.(\d+)\.(\d+)$/.exec(version ?? "");
  if (!match) throw new Error(`invalid exact Node version in eas.json: ${version}`);
  return match.slice(1).map(Number);
};
const compareVersions = (left, right) => {
  for (let index = 0; index < 3; index += 1) {
    if (left[index] !== right[index]) return left[index] - right[index];
  }
  return 0;
};
if (compareVersions(parseVersion(configuredNode), parseVersion("22.13.0")) < 0) {
  throw new Error("EAS Node must support the builder's pnpm 11 runtime (Node >=22.13.0)");
}
const satisfiesNodeRange = (version, range) => {
  const actual = parseVersion(version);
  return range.split("||").some((alternative) => {
    const requirement = alternative.trim();
    const match = /^(\^|>=)\s*(\d+\.\d+\.\d+)$/.exec(requirement);
    if (!match) return false;
    const minimum = parseVersion(match[2]);
    if (match[1] === "^") {
      return actual[0] === minimum[0] && compareVersions(actual, minimum) >= 0;
    }
    return compareVersions(actual, minimum) >= 0;
  });
};
for (const dependency of ["react-native", "metro", "vite", "rolldown"]) {
  const engine = lockfile.packages?.[`node_modules/${dependency}`]?.engines?.node;
  if (!engine) throw new Error(`${dependency} must declare its Node engine in package-lock.json`);
  if (!satisfiesNodeRange(configuredNode, engine)) {
    throw new Error(
      `EAS Node ${configuredNode} does not satisfy ${dependency}'s locked engine ${engine}`,
    );
  }
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
