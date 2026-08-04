import { existsSync, readdirSync, readFileSync, rmSync } from "node:fs";
import { join } from "node:path";
import { spawnSync } from "node:child_process";

for (const path of ["dist", ".expo", "node_modules/.cache/metro"]) {
  rmSync(path, { force: true, recursive: true });
}

const result = spawnSync(process.platform === "win32" ? "npx.cmd" : "npx", ["expo", "export", "-p", "web"], {
  stdio: "inherit",
});
if (result.status !== 0) process.exit(result.status ?? 1);
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
