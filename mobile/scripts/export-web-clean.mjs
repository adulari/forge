import { existsSync, readdirSync, readFileSync, rmSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { spawnSync } from "node:child_process";

for (const path of ["dist", ".expo", "node_modules/.cache/metro"]) {
  rmSync(path, { force: true, recursive: true });
}

// Run the installed Expo CLI with this Node binary rather than through `npx`. Since the
// CVE-2024-27980 mitigation, Node refuses to spawn a `.cmd` shim without `shell: true` and
// spawnSync reports EINVAL in `result.error` with a null status — which is how the windows
// desktop leg of v2.13.0 died in 50ms with no output at all.
const require = createRequire(import.meta.url);
const manifestPath = require.resolve("expo/package.json");
const { bin } = JSON.parse(readFileSync(manifestPath, "utf8"));
const expoCli = join(dirname(manifestPath), typeof bin === "string" ? bin : bin.expo);

const result = spawnSync(process.execPath, [expoCli, "export", "-p", "web"], {
  stdio: "inherit",
});
if (result.error) {
  // A failed spawn leaves status null, so exiting on status alone printed nothing at all.
  console.error(`Expo export could not be started (${expoCli}): ${result.error.message}`);
  process.exit(1);
}
if (result.signal) {
  console.error(`Expo export was killed by signal ${result.signal}`);
  process.exit(1);
}
if (result.status !== 0) {
  console.error(`Expo export failed with exit code ${result.status}`);
  process.exit(result.status ?? 1);
}
if (!existsSync("dist")) throw new Error("Expo export did not create mobile/dist");

const javascript = [];
function collect(path) {
  for (const entry of readdirSync(path, { withFileTypes: true })) {
    const child = join(path, entry.name);
    if (entry.isDirectory()) collect(child);
    else if (/\.(js|html)$/.test(entry.name)) javascript.push(readFileSync(child, "utf8"));
  }
}
collect("dist");
const bundle = javascript.join("\n");
const runtimeGateCount = (bundle.match(/perf_enabled/g) ?? []).length;
const fixtureGateCount = (bundle.match(/perf_fixture_enabled/g) ?? []).length;
if (runtimeGateCount < 1 || fixtureGateCount < 2) {
  throw new Error(`Perf dump and fixture gates are not both runtime-gated (perf=${runtimeGateCount}, fixture=${fixtureGateCount})`);
}
if (bundle.includes("EXPO_PUBLIC_PERF_FIXTURE")) {
  throw new Error("Build-time EXPO_PUBLIC_PERF_FIXTURE gate remains in emitted bundle");
}
console.log(`Clean export verified: ${javascript.length} JS/HTML files, perf=${runtimeGateCount}, fixture=${fixtureGateCount} runtime gates`);
