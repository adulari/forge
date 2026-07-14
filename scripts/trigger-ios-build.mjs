#!/usr/bin/env node
// Trigger an Xcode Cloud build for the iOS app AND (optionally) assign the resulting TestFlight
// build to its beta group(s) — in one step, so testers actually see the build without a manual
// "add to group" click.
//
// Why this exists: the Xcode Cloud workflow is manual-only (its branch start condition points at a
// sentinel branch, `ios-release-manual-only`, so a normal push does NOT build). Triggering a build
// via the App Store Connect API requires temporarily pointing the workflow at `main`, POSTing a
// ciBuildRun, then restoring the sentinel. Separately, Xcode Cloud uploads the build but never
// assigns it to a beta group, so testers see nothing until scripts/testflight-assign-group.mjs
// runs. Doing the trigger without the assign is what left builds #68/#69/#70 stranded. This script
// chains both: trigger, then (if TESTFLIGHT_GROUPS is set) wait for processing and assign.
//
// Env:
//   ASC_KEY_ID          App Store Connect API key id                (required)
//   ASC_ISSUER_ID       ASC API issuer id                           (required)
//   ASC_API_PRIVATE_KEY contents of the .p8 private key             (required)
//   WORKFLOW_ID         Xcode Cloud ciWorkflow id                   (default = Forge's)
//   BUILD_BRANCH        branch to build                             (default main)
//   TESTFLIGHT_GROUPS   comma-separated beta group name(s)          (optional; if set, auto-assign)
//   BUNDLE_ID           app bundle id                               (default dev.adulari.forge)
// Any env accepted by testflight-assign-group.mjs (POLL_TIMEOUT_SEC, etc.) is passed through.
//
// Usage: ASC_KEY_ID=... ASC_ISSUER_ID=... ASC_API_PRIVATE_KEY="$(cat AuthKey_XXX.p8)" \
//        TESTFLIGHT_GROUPS=Testers node scripts/trigger-ios-build.mjs

import { createSign } from "node:crypto";
import { request } from "node:https";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const {
  ASC_KEY_ID,
  ASC_ISSUER_ID,
  ASC_API_PRIVATE_KEY,
  WORKFLOW_ID = "C68BAA13-19B5-4C45-B4D7-C947DB601DB6",
  BUILD_BRANCH = "main",
  TESTFLIGHT_GROUPS = "",
} = process.env;

function die(msg) {
  console.error(`✗ ${msg}`);
  process.exit(1);
}
for (const [k, v] of Object.entries({ ASC_KEY_ID, ASC_ISSUER_ID, ASC_API_PRIVATE_KEY })) {
  if (!v) die(`missing required env ${k}`);
}

function b64url(buf) {
  return Buffer.from(buf).toString("base64").replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}
function mintToken() {
  const header = { alg: "ES256", kid: ASC_KEY_ID, typ: "JWT" };
  const now = Math.floor(Date.now() / 1000);
  const payload = { iss: ASC_ISSUER_ID, iat: now, exp: now + 19 * 60, aud: "appstoreconnect-v1" };
  const signingInput = `${b64url(JSON.stringify(header))}.${b64url(JSON.stringify(payload))}`;
  const signer = createSign("SHA256");
  signer.update(signingInput);
  const sig = signer.sign({ key: ASC_API_PRIVATE_KEY, dsaEncoding: "ieee-p1363" });
  return `${signingInput}.${b64url(sig)}`;
}
function api(method, path, body) {
  const payload = body ? JSON.stringify(body) : null;
  const options = {
    method,
    hostname: "api.appstoreconnect.apple.com",
    path,
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

function runAssign() {
  return new Promise((resolve) => {
    const here = dirname(fileURLToPath(import.meta.url));
    const script = join(here, "testflight-assign-group.mjs");
    // The build was just POSTed; only assign a build uploaded at/after ~2 min ago so we target THIS
    // build, not a stale one already on TestFlight.
    const uploadedAfter = new Date(Date.now() - 2 * 60 * 1000).toISOString();
    const child = spawn(process.execPath, [script], {
      stdio: "inherit",
      env: { ...process.env, UPLOADED_AFTER: process.env.UPLOADED_AFTER || uploadedAfter },
    });
    child.on("exit", (code) => resolve(code ?? 0));
    child.on("error", (e) => {
      console.error(`✗ could not spawn assign script: ${e.message}`);
      resolve(1);
    });
  });
}

async function main() {
  // 1. Read the workflow's current start condition so we can restore it exactly.
  const wf = await api("GET", `/v1/ciWorkflows/${WORKFLOW_ID}`);
  const originalBranch = wf.data?.attributes?.branchStartCondition ?? null;
  console.log("current branchStartCondition:", JSON.stringify(originalBranch));

  // 2. Point it at the build branch so a manual build is allowed.
  const patchTo = {
    data: {
      type: "ciWorkflows",
      id: WORKFLOW_ID,
      attributes: {
        branchStartCondition: {
          source: { isAllMatch: false, patterns: [{ pattern: BUILD_BRANCH, isPrefix: false }] },
          filesAndFoldersRule: originalBranch?.filesAndFoldersRule ?? null,
          autoCancel: originalBranch?.autoCancel ?? true,
        },
      },
    },
  };
  await api("PATCH", `/v1/ciWorkflows/${WORKFLOW_ID}`, patchTo);
  console.log(`patched branchStartCondition -> ${BUILD_BRANCH}`);
  await sleep(3000);

  // 3. Kick the build. Restore the sentinel no matter what.
  let triggered = false;
  try {
    const run = await api("POST", "/v1/ciBuildRuns", {
      data: { type: "ciBuildRuns", relationships: { workflow: { data: { type: "ciWorkflows", id: WORKFLOW_ID } } } },
    });
    triggered = true;
    console.log(`✓ triggered build run ${run.data?.id} (number ${run.data?.attributes?.number})`);
  } catch (e) {
    console.error(`✗ POST ciBuildRuns failed: ${e.status} ${JSON.stringify(e.json)?.slice(0, 300)}`);
  } finally {
    if (originalBranch) {
      await api("PATCH", `/v1/ciWorkflows/${WORKFLOW_ID}`, {
        data: { type: "ciWorkflows", id: WORKFLOW_ID, attributes: { branchStartCondition: originalBranch } },
      }).then(() => console.log("restored original branchStartCondition")).catch((e) => console.error(`✗ restore failed: ${e.message}`));
    }
  }
  if (!triggered) process.exit(1);

  // 4. Auto-assign the freshly-built build to its beta group(s), if configured.
  if (TESTFLIGHT_GROUPS.trim()) {
    console.log(`\nwaiting for the build to process, then assigning to: ${TESTFLIGHT_GROUPS}`);
    const code = await runAssign();
    process.exit(code);
  } else {
    console.log("\nTESTFLIGHT_GROUPS unset — build triggered but NOT assigned to a group.");
    console.log("Set TESTFLIGHT_GROUPS (e.g. Testers) to auto-assign, or run scripts/testflight-assign-group.mjs.");
  }
}

main().catch((e) => die(e.message));
