const CACHE = "forge-remote-v5";
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
