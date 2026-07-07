// Forge PWA service worker (ARCHITECTURE.md §5 Web target, FEATURES.md §1.1/§5;
// BUILD_ORDER.md T4.3). Plain JS, not compiled/type-checked (see tsconfig.json's
// exclude) — Expo copies `public/` verbatim into the web export root, so this
// file is served as-is at `/sw.js` and registered by `src/lib/push/push.web.ts`.
//
// Two jobs:
//   1. `push`            — show a notification for a daemon-sent PushMessage
//      (`{kind, session, title, body, seq}` — see crates/forge-cli/src/push.rs
//      `PushMessage::payload_json`). Decision kinds ("permission") get
//      Allow/Deny actions; others (question/done/failed) are tap-to-open.
//   2. `notificationclick` — Allow/Deny POSTs `{base}/api/answer` directly
//      (the daemon built this route exactly for notification actions, no WS
//      needed); a body tap focuses/opens `/session/{id}`.
//
// The daemon's `baseUrl` (scheme+host+port+token) isn't in the push payload
// and isn't reachable from `localStorage` inside a service worker, so
// `push.web.ts` mirrors the active server's baseUrl into IndexedDB whenever
// push is (re)enabled; this worker reads that same record to answer prompts.
/* eslint-disable no-restricted-globals */

const DB_NAME = "forge-push";
const STORE_NAME = "kv";
const BASE_URL_KEY = "baseUrl";

function openDb() {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open(DB_NAME, 1);
    req.onupgradeneeded = () => {
      req.result.createObjectStore(STORE_NAME);
    };
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

async function getStoredBaseUrl() {
  try {
    const db = await openDb();
    return await new Promise((resolve, reject) => {
      const tx = db.transaction(STORE_NAME, "readonly");
      const req = tx.objectStore(STORE_NAME).get(BASE_URL_KEY);
      req.onsuccess = () => resolve(req.result || null);
      req.onerror = () => reject(req.error);
    });
  } catch {
    return null;
  }
}

self.addEventListener("install", () => {
  self.skipWaiting();
});

self.addEventListener("activate", (event) => {
  event.waitUntil(self.clients.claim());
});

self.addEventListener("push", (event) => {
  let payload = {};
  try {
    payload = event.data ? event.data.json() : {};
  } catch {
    payload = {};
  }

  const kind = payload.kind || "done";
  const title = payload.title || "Forge";
  const body = payload.body || "";
  const session = payload.session || "";
  const seq = typeof payload.seq === "number" ? payload.seq : 0;

  const actions =
    kind === "permission"
      ? [
          { action: "allow", title: "Allow" },
          { action: "deny", title: "Deny" },
        ]
      : [];

  event.waitUntil(
    self.registration.showNotification(title, {
      body,
      tag: session || undefined,
      renotify: true,
      data: { kind, session, seq },
      actions,
    }),
  );
});

async function answerPrompt(session, seq, allow) {
  const base = await getStoredBaseUrl();
  if (!base || !session) return;
  try {
    await fetch(`${base}/api/answer`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ session, seq, allow }),
    });
  } catch {
    // best-effort — the user can still open the app and answer the prompt
    // directly if the daemon was unreachable from here
  }
}

async function focusOrOpenSession(sessionId) {
  const path = sessionId ? `/session/${sessionId}` : "/";
  const windows = await self.clients.matchAll({ type: "window", includeUncontrolled: true });

  for (const client of windows) {
    if (new URL(client.url).pathname === path && "focus" in client) {
      return client.focus();
    }
  }
  for (const client of windows) {
    if ("focus" in client) {
      await client.focus();
      if ("navigate" in client) return client.navigate(path);
      return;
    }
  }
  if (self.clients.openWindow) return self.clients.openWindow(path);
  return undefined;
}

self.addEventListener("notificationclick", (event) => {
  const data = event.notification.data || {};
  const action = event.action;
  event.notification.close();

  if (action === "allow" || action === "deny") {
    event.waitUntil(answerPrompt(data.session, data.seq, action === "allow"));
    return;
  }

  event.waitUntil(focusOrOpenSession(data.session));
});
