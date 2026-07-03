"use strict";
const BASE = "__BASE__";
const PROTO = 4;
const $ = (id) => document.getElementById(id);
let ws = null, dead = false, notif = false, curSeq = 0, retries = 0, curOverlay = null;
// The /copy payload stashed outside the DOM (it can be large / contain anything).
let copyPayload = "";
let prev = { busy: false, prompt: false, question: false };

function connect() {
  if (dead) return;
  const scheme = location.protocol === "https:" ? "wss://" : "ws://";
  ws = new WebSocket(scheme + location.host + BASE + "/ws");
  ws.onopen = () => { retries = 0; $("conn").textContent = "● connected"; };
  ws.onmessage = (e) => {
    let s; try { s = JSON.parse(e.data); } catch { return; }
    render(s);
    if (s.closed) { dead = true; $("conn").textContent = "remote control turned off — reconnect to the TUI"; ws.close(); }
  };
  ws.onclose = () => {
    if (dead) return;
    retries++;
    // After ~12s of failures the session is almost certainly gone (the server dies with the
    // TUI) — say so instead of an infinite "reconnecting…", and back off to a slow retry.
    $("conn").textContent = retries > 8
      ? "session unreachable — reopen /remote from the TUI for a fresh link"
      : "reconnecting…";
    setTimeout(connect, retries > 8 ? 10000 : 1500);
  };
  ws.onerror = () => ws.close();
}
function send(obj) { if (ws && ws.readyState === 1) ws.send(JSON.stringify(obj)); }

function render(s) {
  if (s.protocol && s.protocol !== PROTO) {
    $("banner").style.display = "block";
    $("banner").textContent = s.protocol > PROTO
      ? "A newer Forge is running — refresh this page to update the remote UI."
      : "This page is newer than the running Forge — restart Forge in the terminal, then reload.";
  }
  curSeq = s.prompt_seq || 0;
  curOverlay = s.overlay || null;
  copyPayload = s.copy_text || "";
  $("dot").className = "dot" + (s.busy ? " busy" : "");
  $("tier").textContent = s.tier ? "[" + s.tier + "]" : "—";
  $("model").textContent = s.model || "—";
  $("cost").textContent = "$" + (s.cost_usd || 0).toFixed(4);
  $("temper").textContent = s.temper ? "◆ " + s.temper : "";
  $("sid").textContent = (s.session_id || "").slice(0, 8) || "—";
  $("cwd").textContent = baseName(s.cwd) || "—";
  $("expo").textContent = s.exposure || "—";
  $("expo").className = "badge" + ((s.exposure || "").indexOf("public") === 0 ? " pub" : "");
  if (s.context_tokens > 0) {
    const lim = s.context_limit ? "/" + fmt(s.context_limit) : "";
    $("ctx").textContent = "◷ " + fmt(s.context_tokens) + lim;
  } else { $("ctx").textContent = ""; }

  renderTranscript(s);
  renderTasks(s);
  renderAgents(s);
  renderOverlay(s);
  renderActions(s);
  notifyTransitions(s);
}

function renderTranscript(s) {
  const t = $("transcript");
  // Exact content signature — a length-based check missed equal-length ring
  // rotations/replacements and left stale lines on screen.
  const body = (s.transcript || []).join("\n") + (s.streaming ? "\n" + s.streaming : "");
  if (t._sig === body) return; // unchanged
  const nearBottom = t.scrollHeight - t.scrollTop - t.clientHeight < 80;
  t.innerHTML = "";
  (s.transcript || []).forEach(line => { const d = document.createElement("div"); d.textContent = line; t.appendChild(d); });
  if (s.streaming) { const d = document.createElement("div"); d.className = "stream"; d.textContent = s.streaming; t.appendChild(d); }
  if (nearBottom) t.scrollTop = t.scrollHeight;
  t._sig = body;
}

// Rebuild `el`'s contents via `fill`, but preserve scroll position across the rebuild — a plain
// `innerHTML = ""` resets scrollTop to 0, which would yank the view back to the top every time a
// new snapshot arrives (e.g. a subagent's `last` line updating mid-stream) while someone is
// scrolled up reading earlier entries. Skips the rebuild entirely when `sig` is unchanged.
function rebuildPreservingScroll(el, sig, fill) {
  if (el._sig === sig) return;
  el._sig = sig;
  const nearBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
  const scrollTop = el.scrollTop;
  fill();
  el.scrollTop = nearBottom ? el.scrollHeight : scrollTop;
}

function renderTasks(s) {
  const tasks = s.tasks || [];
  $("tc").textContent = tasks.length ? tasks.filter(x => x.status === "done").length + "/" + tasks.length : "";
  const el = $("tasks");
  rebuildPreservingScroll(el, JSON.stringify(tasks), () => {
    if (!tasks.length) { el.innerHTML = '<div class="empty">no tasks yet</div>'; return; }
    el.innerHTML = "";
    tasks.forEach(t => {
      const d = document.createElement("div"); d.className = "task " + t.status;
      const g = t.status === "done" ? "●" : (t.status === "in_progress" ? "◐" : "○");
      d.innerHTML = '<span class="g">' + g + '</span><span>' + esc(t.title) + '</span>';
      el.appendChild(d);
    });
  });
}

function renderAgents(s) {
  const subs = s.subagents || [];
  $("ac").textContent = subs.length ? "" + subs.length : "";
  const el = $("agents");
  rebuildPreservingScroll(el, JSON.stringify(subs), () => {
    if (!subs.length) { el.innerHTML = '<div class="empty">no subagents running</div>'; return; }
    el.innerHTML = "";
    subs.forEach(a => {
      const d = document.createElement("div"); d.className = "agent" + (a.done ? " done" : "");
      const head = esc(a.agent || "agent") + (a.model ? " · " + esc(a.model) : "") + (a.done ? " · done $" + (a.cost || 0).toFixed(4) : "");
      d.innerHTML = '<div class="ah">' + (a.done ? "✓ " : "▸ ") + head + '</div>' +
        '<div class="at">' + esc(a.task || "") + '</div>' +
        '<div class="al">' + esc(a.last || "") + '</div>';
      el.appendChild(d);
    });
  });
}

// The generic overlay panel: whatever modal surface owns the TUI keyboard (palette / any picker
// / config / usage / mesh / workflow) is mirrored here — tappable rows, a filter box, a
// free-text box, or a pre-rendered text body. All input goes back as overlay_* / key inputs, so
// the server drives the SAME code path a local keystroke takes.
function renderOverlay(s) {
  const o = s.overlay;
  const box = $("overlay");
  if (!o) {
    if (!box.hidden) { box.hidden = true; $("orows")._sig = ""; }
    return;
  }
  box.hidden = false;
  $("otitle").textContent = o.title || o.kind || "—";
  const f = $("ofilter");
  const hasFilter = o.filter !== null && o.filter !== undefined;
  f.hidden = !hasFilter;
  // Never clobber the filter box while the user is typing in it — the server echo lags a frame.
  if (hasFilter && document.activeElement !== f) f.value = o.filter;
  $("ofreebar").hidden = !o.free_text;
  const b = $("obody");
  b.hidden = !o.body;
  if (o.body) b.textContent = o.body;
  const rowsEl = $("orows");
  const sig = JSON.stringify([o.kind, o.rows]);
  if (rowsEl._sig === sig) return;
  rowsEl._sig = sig;
  rowsEl.innerHTML = "";
  let lastGroup = null;
  (o.rows || []).forEach(r => {
    if (r.group && r.group !== lastGroup) {
      const g = document.createElement("div");
      g.className = "ogroup"; g.textContent = r.group;
      rowsEl.appendChild(g);
      lastGroup = r.group;
    }
    const d = document.createElement("button");
    d.className = "orow" + (r.selected ? " sel" : "");
    d.dataset.id = r.id;
    d.innerHTML = "<b>" + esc(r.label) + "</b>" + (r.detail ? "<span>" + esc(r.detail) + "</span>" : "");
    rowsEl.appendChild(d);
  });
  const sel = rowsEl.querySelector(".orow.sel");
  if (sel) sel.scrollIntoView({ block: "nearest" });
}

function renderActions(s) {
  const a = $("actions");
  const queued = s.queued || [];
  const notes = s.notes || [];
  // Rebuild ONLY when the actionable state changed. This area holds live buttons + a free-text
  // input; rebuilding it on every snapshot (streaming updates arrive continuously while busy)
  // destroyed the nodes mid-tap ("tapped Allow, nothing happened") and wiped typed answers.
  const sig = JSON.stringify([queued, notes, s.permission_prompt, s.question,
    s.question_options, s.question_allow_other, s.prompt_seq, s.copy_text]);
  if (a._sig === sig) return;
  a._sig = sig;
  let h = queued.length
    ? '<div class="queued">' + queued.map(q => "⏳ queued: " + esc(q)).join("<br>") + '</div>'
    : "";
  if (notes.length) {
    h += '<div class="queued">' + notes.map(esc).join("<br>") + '</div>';
  }
  if (s.copy_text) {
    h += '<div class="copybar"><span class="cinfo">📋 response ready (' + s.copy_text.length +
      ' chars)</span><button class="send" data-act="copy">Copy here</button></div>';
  }
  if (s.permission_prompt) {
    h += '<div class="prompt"><span class="q">⚠ ' + esc(s.permission_prompt) +
      '</span></div><div class="bar"><button class="y" data-act="allow" data-yes="1">Allow</button>' +
      '<button class="n" data-act="allow" data-yes="0">Deny</button></div>';
  } else if (s.question) {
    h += '<div class="prompt"><span class="q">❓ ' + esc(s.question) + '</span></div>';
    const opts = s.question_options || [];
    if (opts.length) {
      h += '<div class="opts">';
      opts.forEach((o, i) => {
        h += '<button class="opt" data-act="pick" data-n="' + (i + 1) + '"><b>' + esc(o.label) + '</b>' +
          (o.description ? '<span>' + esc(o.description) + '</span>' : '') + '</button>';
      });
      h += '</div>';
    }
    if (!opts.length || s.question_allow_other) {
      h += '<div class="bar"><input type="text" id="ans" placeholder="answer…" enterkeyhint="done">' +
        '<button class="send" data-act="answer">Answer</button></div>';
    }
  }
  a.innerHTML = h;
}

function notifyTransitions(s) {
  const pPrompt = !!s.permission_prompt, pQuestion = !!s.question;
  if (pPrompt && !prev.prompt) maybeNotify("Forge needs permission", s.permission_prompt);
  if (pQuestion && !prev.question) maybeNotify("Forge has a question", s.question);
  if (!s.busy && prev.busy && !pPrompt && !pQuestion) maybeNotify("Forge — turn complete", lastLine(s));
  prev = { busy: !!s.busy, prompt: pPrompt, question: pQuestion };
}
function lastLine(s) { const t = s.transcript || []; return t.length ? t[t.length - 1] : ""; }

function fmt(n) { if (n >= 1e6) return (n/1e6).toFixed(1)+"M"; if (n >= 1e3) return (n/1e3).toFixed(1)+"k"; return ""+n; }
function esc(s) { return (s||"").replace(/[&<>]/g, c => ({"&":"&amp;","<":"&lt;",">":"&gt;"}[c])); }
function baseName(p) { if (!p) return ""; const parts = (""+p).replace(/[\\/]+$/, "").split(/[\\/]/); return parts[parts.length-1] || p; }

function submit() { const v = $("prompt").value; if (!v.trim()) return; send({kind:"prompt", text:v}); $("prompt").value=""; }
$("send").onclick = submit;
$("prompt").addEventListener("keydown", e => { if (e.key === "Enter") { e.preventDefault(); submit(); } });
$("stop").onclick = () => send({kind:"interrupt"});

// Quick-command chips (all functional server-side: /model, /mode, /help open pickers/the palette
// that render in the overlay panel above).
document.querySelectorAll(".chip[data-cmd]").forEach(b => {
  b.onclick = () => send({ kind: "prompt", text: b.dataset.cmd });
});

// Actions area (permission / question / copy) is rebuilt from snapshots, so its buttons are
// handled by delegation — no inline handlers (the CSP forbids them).
$("actions").addEventListener("click", (e) => {
  const b = e.target.closest("[data-act]");
  if (!b) return;
  // Answers echo the prompt_seq their buttons were rendered from, so a stale tap can never
  // resolve a newer prompt that replaced this one.
  if (b.dataset.act === "allow") send({ kind: "allow", yes: b.dataset.yes === "1", seq: curSeq });
  else if (b.dataset.act === "pick") send({ kind: "answer", text: b.dataset.n, seq: curSeq });
  else if (b.dataset.act === "answer") {
    const v = ($("ans") && $("ans").value) || "";
    if (v.trim()) send({ kind: "answer", text: v, seq: curSeq });
  } else if (b.dataset.act === "copy") {
    copyText(copyPayload);
  }
});

function copyText(text) {
  if (!text) return;
  if (navigator.clipboard && navigator.clipboard.writeText) {
    navigator.clipboard.writeText(text).catch(() => fallbackCopy(text));
  } else { fallbackCopy(text); }
}
function fallbackCopy(text) {
  const ta = document.createElement("textarea");
  ta.value = text; ta.style.position = "fixed"; ta.style.opacity = "0";
  document.body.appendChild(ta); ta.select();
  try { document.execCommand("copy"); } catch (e) {}
  ta.remove();
}

// Overlay events: row taps select-by-id (the server moves its cursor there and synthesizes
// Enter through the same key path a local Enter takes), the filter box replaces the overlay's
// query, ✕ cancels, and the free-text box sets the pending value then commits with Enter.
$("ocancel").onclick = () => send({ kind: "overlay_cancel" });
$("orows").addEventListener("click", (e) => {
  const b = e.target.closest(".orow");
  if (b) send({ kind: "overlay_select", id: b.dataset.id });
});
$("ofilter").addEventListener("input", () => send({ kind: "overlay_filter", text: $("ofilter").value }));
function submitFree() {
  send({ kind: "overlay_filter", text: $("ofree").value });
  send({ kind: "key", key: "Enter" });
  $("ofree").value = "";
}
$("ofreeok").onclick = submitFree;
$("ofree").addEventListener("keydown", e => { if (e.key === "Enter") { e.preventDefault(); submitFree(); } });

// Desktop keyboard parity while an overlay is open: arrows/Enter/Esc/Tab/paging go to the host
// as named keys. Text inputs keep their own editing keys.
document.addEventListener("keydown", (e) => {
  if (!curOverlay) return;
  const t = e.target;
  const inInput = t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA");
  if (e.key === "Escape") { e.preventDefault(); send({ kind: "overlay_cancel" }); return; }
  if (inInput && t.id === "ofree") return; // its own Enter handler commits
  if (inInput && !["ArrowUp", "ArrowDown", "Enter", "Tab"].includes(e.key)) return;
  const map = { ArrowUp: "Up", ArrowDown: "Down", PageUp: "PageUp", PageDown: "PageDown",
    Home: "Home", End: "End", Enter: "Enter", Tab: "Tab" };
  const named = map[e.key];
  if (named) { e.preventDefault(); send({ kind: "key", key: named }); }
});

// Tabs
document.querySelectorAll(".tab").forEach(b => b.onclick = () => {
  document.querySelectorAll(".tab").forEach(x => x.classList.remove("on"));
  b.classList.add("on");
  const which = b.dataset.tab;
  $("transcript").hidden = which !== "chat";
  $("tasks").hidden = which !== "tasks";
  $("agents").hidden = which !== "agents";
});

// Notifications (live, while the page/PWA is open in the background)
function paintBell() { $("bell").textContent = notif ? "🔔" : "🔕"; }
$("bell").onclick = () => {
  if (!("Notification" in window)) { $("bell").title = "notifications unsupported"; return; }
  if (Notification.permission === "granted") { notif = !notif; paintBell(); return; }
  Notification.requestPermission().then(p => { notif = (p === "granted"); paintBell(); });
};
function maybeNotify(title, body) {
  if (!(notif && document.hidden && "Notification" in window && Notification.permission === "granted")) return;
  const opts = { body: (body||"").slice(0, 120), icon: BASE + "/icon.svg", tag: "forge-remote" };
  // Android Chrome throws on the page-context Notification constructor — notifications must go
  // through the service worker registration there. Fall back to the constructor elsewhere.
  if (navigator.serviceWorker && navigator.serviceWorker.ready) {
    navigator.serviceWorker.ready
      .then(r => r.showNotification(title, opts))
      .catch(() => { try { new Notification(title, opts); } catch (e) {} });
  } else {
    try { new Notification(title, opts); } catch (e) {}
  }
}

// PWA: register the token-scoped service worker so the page installs to a home screen.
if ("serviceWorker" in navigator) {
  navigator.serviceWorker.register(BASE + "/sw.js", { scope: BASE + "/" }).catch(() => {});
}

$("prompt").focus();
connect();
