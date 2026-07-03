const CACHE = "forge-remote-v6";
const ENDED = `<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Forge remote - session ended</title>
<style>body{background:#16161c;color:#d8d8e0;font:16px/1.6 -apple-system,system-ui,sans-serif;
display:flex;align-items:center;justify-content:center;height:100vh;margin:0;text-align:center;
padding:24px}b{color:#ff913c}</style></head><body><div><b>⚒ Forge</b><br><br>
This remote session has ended.<br>Each session gets a fresh link: reopen <b>/remote</b> in the
Forge TUI and scan the new QR code (or open the new URL).</div></body></html>`;
self.addEventListener("install", () => self.skipWaiting());
self.addEventListener("activate", (e) => e.waitUntil(self.clients.claim()));
self.addEventListener("fetch", (e) => {
  const req = e.request;
  if (req.method !== "GET") return;
  // Live data (history pagination) must never be answered from cache — a cached page would
  // hide new turns. Let the browser hit the network directly.
  if (new URL(req.url).pathname.includes("/api/")) return;
  if (req.mode === "navigate") {
    e.respondWith(fetch(req).catch(() =>
      new Response(ENDED, { headers: { "Content-Type": "text/html; charset=utf-8" } })));
    return;
  }
  e.respondWith(
    fetch(req).then((res) => {
      const copy = res.clone();
      caches.open(CACHE).then((c) => c.put(req, copy)).catch(() => {});
      return res;
    }).catch(() => caches.match(req))
  );
});

// --- Actionable Web Push (forge serve) ------------------------------------------------------
// The daemon encrypts every payload end-to-end (RFC 8291); by the time it reaches this handler
// the browser has already decrypted it. Payload: { kind, session, title, body, seq } where kind
// is "permission" | "question" | "done" | "failed". A permission push carries Allow/Deny
// actions the notificationclick handler answers DIRECTLY over POST api/answer — no page needed,
// so the agent can be unblocked from the lock screen.
self.addEventListener("push", (e) => {
  let d = {};
  try { d = e.data ? e.data.json() : {}; } catch (err) {}
  const kind = d.kind || "";
  const head = kind === "permission" ? "Forge needs permission"
    : kind === "question" ? "Forge has a question"
    : kind === "failed" ? "Forge — turn failed"
    : "Forge — turn complete";
  const title = d.title ? head + " · " + d.title : head;
  const actions = kind === "permission"
    ? [{ action: "approve", title: "Allow" }, { action: "deny", title: "Deny" }]
    : [{ action: "open", title: "Open" }];
  e.waitUntil(self.registration.showNotification(title, {
    body: d.body || "",
    icon: new URL("icon.svg", self.registration.scope).href,
    // One notification per pending decision: a replaced prompt overwrites the stale card
    // instead of stacking; completions collapse per session.
    tag: "forge-push:" + (d.session || "") + ":" + (kind === "done" || kind === "failed" ? "turn" : "ask"),
    data: d,
    actions,
  }));
});

self.addEventListener("notificationclick", (e) => {
  const d = e.notification.data || {};
  e.notification.close();
  if ((e.action === "approve" || e.action === "deny") && d.kind === "permission") {
    // Answer straight from the notification — the seq the daemon sent rides back, so a stale
    // tap on a replaced prompt is a server-side 409 no-op, exactly like the WS path.
    e.waitUntil(fetch(new URL("api/answer", self.registration.scope).href, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ session: d.session || "", seq: d.seq || 0, allow: e.action === "approve" }),
    }).catch(() => {}));
    return;
  }
  // Default click / "Open": focus an existing page inside our scope, else open one.
  e.waitUntil(self.clients.matchAll({ type: "window", includeUncontrolled: true }).then((list) => {
    for (const c of list) {
      if (c.url.startsWith(self.registration.scope) && "focus" in c) return c.focus();
    }
    return self.clients.openWindow(self.registration.scope);
  }));
});
