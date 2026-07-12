#!/usr/bin/env node
// Auto-assign the newest processed TestFlight build to one or more internal beta groups,
// via the App Store Connect API. Xcode Cloud uploads builds to TestFlight (audience
// INTERNAL_ONLY) but never assigns them to a group, so no tester sees them until this runs.
// Dependency-free: ES256 JWT + ASC REST over Node's built-in crypto/https.
//
// Env:
//   ASC_KEY_ID          App Store Connect API key id      (required)
//   ASC_ISSUER_ID       ASC API issuer id                 (required)
//   ASC_API_PRIVATE_KEY contents of the .p8 private key    (required)
//   TESTFLIGHT_GROUPS   comma-separated beta group names   (required, e.g. "Internal")
//   BUNDLE_ID           app bundle id                      (default dev.adulari.forge)
//   APP_VERSION         marketing version to match         (optional; default = latest)
//   UPLOADED_AFTER      ISO-8601 instant; only assign a     (optional; the workflow sets this to
//                       build uploaded at/after it — so we   its own start time so we wait for the
//                       wait for THIS run's build, not a     build THIS push produced, not a stale
//                       stale pre-existing one               one already on TestFlight)
//   POLL_TIMEOUT_SEC    how long to wait for a processed    (default 2700 = 45m)
//                       build to appear (Xcode Cloud build + Apple processing)
//   POLL_INTERVAL_SEC   seconds between polls               (default 60)

import { createSign } from "node:crypto";
import { request } from "node:https";

const {
  ASC_KEY_ID,
  ASC_ISSUER_ID,
  ASC_API_PRIVATE_KEY,
  TESTFLIGHT_GROUPS,
  BUNDLE_ID = "dev.adulari.forge",
  APP_VERSION = "",
  UPLOADED_AFTER = "",
  POLL_TIMEOUT_SEC = "2700",
  POLL_INTERVAL_SEC = "60",
} = process.env;

function die(msg) {
  console.error(`✗ ${msg}`);
  process.exit(1);
}

for (const [k, v] of Object.entries({ ASC_KEY_ID, ASC_ISSUER_ID, ASC_API_PRIVATE_KEY, TESTFLIGHT_GROUPS })) {
  if (!v) die(`missing required env ${k}`);
}

const groupNames = TESTFLIGHT_GROUPS.split(",").map((s) => s.trim()).filter(Boolean);
const uploadedAfterMs = UPLOADED_AFTER ? Date.parse(UPLOADED_AFTER) : 0;
if (UPLOADED_AFTER && Number.isNaN(uploadedAfterMs)) die(`UPLOADED_AFTER is not a valid ISO instant: ${UPLOADED_AFTER}`);

function b64url(buf) {
  return Buffer.from(buf).toString("base64").replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

// ASC tokens must live <= 20 minutes; mint a fresh one per run (a long poll may outlive one, so
// re-mint on demand via a getter rather than caching a single token).
function mintToken() {
  const header = { alg: "ES256", kid: ASC_KEY_ID, typ: "JWT" };
  const now = Math.floor(Date.now() / 1000);
  const payload = { iss: ASC_ISSUER_ID, iat: now, exp: now + 19 * 60, aud: "appstoreconnect-v1" };
  const signingInput = `${b64url(JSON.stringify(header))}.${b64url(JSON.stringify(payload))}`;
  const signer = createSign("SHA256");
  signer.update(signingInput);
  // ASC requires the raw (r||s) ECDSA signature, which Node emits with dsaEncoding "ieee-p1363".
  const sig = signer.sign({ key: ASC_API_PRIVATE_KEY, dsaEncoding: "ieee-p1363" });
  return `${signingInput}.${b64url(sig)}`;
}

function api(method, path, body) {
  const payload = body ? JSON.stringify(body) : null;
  const options = {
    method,
    hostname: "api.appstoreconnect.apple.com",
    path: path.startsWith("http") ? path.replace("https://api.appstoreconnect.apple.com", "") : path,
    headers: {
      Authorization: `Bearer ${mintToken()}`,
      Accept: "application/json",
      ...(payload ? { "Content-Type": "application/json", "Content-Length": Buffer.byteLength(payload) } : {}),
    },
  };
  return new Promise((resolve, reject) => {
    const req = request(options, (res) => {
      let data = "";
      res.on("data", (c) => (data += c));
      res.on("end", () => {
        const json = data ? JSON.parse(data) : {};
        if (res.statusCode >= 200 && res.statusCode < 300) resolve(json);
        else reject(Object.assign(new Error(`ASC ${method} ${path} -> ${res.statusCode}`), { status: res.statusCode, json }));
      });
    });
    req.on("error", reject);
    if (payload) req.write(payload);
    req.end();
  });
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function main() {
  // 1. Resolve the app by bundle id.
  const apps = await api("GET", `/v1/apps?filter[bundleId]=${encodeURIComponent(BUNDLE_ID)}&limit=1`);
  const app = apps.data?.[0];
  if (!app) die(`no App Store Connect app for bundle id ${BUNDLE_ID}`);
  const appId = app.id;
  console.log(`app: ${app.attributes?.name} (${appId})`);

  // 2. Resolve the requested beta groups by name.
  const groupsResp = await api("GET", `/v1/betaGroups?filter[app]=${appId}&limit=200`);
  const groups = groupNames.map((name) => {
    const g = (groupsResp.data || []).find((x) => x.attributes?.name === name);
    if (!g) die(`beta group "${name}" not found on this app (have: ${(groupsResp.data || []).map((x) => x.attributes?.name).join(", ") || "none"})`);
    return { id: g.id, name };
  });

  // 3. Poll for a VALID (processed) build for the target version, uploaded at/after the cutoff
  //    (so we assign the build THIS run produced, not a stale one already on TestFlight).
  const versionFilter = APP_VERSION ? `&filter[preReleaseVersion.version]=${encodeURIComponent(APP_VERSION)}` : "";
  const timeoutMs = Number(POLL_TIMEOUT_SEC) * 1000;
  const intervalMs = Number(POLL_INTERVAL_SEC) * 1000;
  const fresh = (b) => !uploadedAfterMs || Date.parse(b.attributes?.uploadedDate || 0) >= uploadedAfterMs;
  const started = Date.now();
  let build = null;
  while (Date.now() - started < timeoutMs) {
    const builds = await api("GET", `/v1/builds?filter[app]=${appId}${versionFilter}&sort=-version&limit=20`);
    const candidate = (builds.data || []).find((b) => b.attributes?.processingState === "VALID" && fresh(b));
    if (candidate) {
      build = candidate;
      break;
    }
    const newest = builds.data?.[0];
    console.log(
      `waiting for a processed build uploaded after ${UPLOADED_AFTER || "(any)"}` +
        (newest ? ` (newest so far: build ${newest.attributes?.version}, ${newest.attributes?.processingState}, uploaded ${newest.attributes?.uploadedDate})` : " (none yet)") +
        ` — ${Math.round((Date.now() - started) / 1000)}s elapsed`,
    );
    await sleep(intervalMs);
  }
  if (!build) die(`timed out after ${POLL_TIMEOUT_SEC}s waiting for a processed build uploaded after ${UPLOADED_AFTER || "(any)"}`);
  console.log(`build: ${build.attributes?.version} (${build.id}) — assigning to: ${groups.map((g) => g.name).join(", ")}`);

  // 4. Assign the build to each group (idempotent: a 409/"already added" is fine).
  for (const g of groups) {
    try {
      await api("POST", `/v1/betaGroups/${g.id}/relationships/builds`, { data: [{ type: "builds", id: build.id }] });
      console.log(`  ✓ added build ${build.attributes?.version} to "${g.name}"`);
    } catch (e) {
      if (e.status === 409) console.log(`  = build already in "${g.name}"`);
      else throw e;
    }
  }
  console.log("✓ TestFlight group assignment complete");
}

main().catch((e) => {
  console.error(e.json ? `${e.message}\n${JSON.stringify(e.json, null, 2)}` : e);
  process.exit(1);
});
