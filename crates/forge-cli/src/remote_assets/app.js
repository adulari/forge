"use strict";
const BASE = "__BASE__";
const PROTO = 6;
const $ = (id) => document.getElementById(id);
let ws = null, dead = false, notif = false, curSeq = 0, retries = 0, curOverlay = null;
// The /copy payload stashed outside the DOM (it can be large / contain anything).
let copyPayload = "";
let prev = { busy: false, prompt: false, question: false };
// v6 multi-session daemon: when the server is `forge serve`, the page addresses ONE of its
// sessions with `?session=<id>` and offers a session list (attach / create / archive). The
// in-chat single-session server has no /api/sessions route — probing it is how the page knows
// which world it's in. The attached session id survives reloads.
const SESS_KEY = "forge-session:" + BASE;
let daemon = false, curSession = sessionStorage.getItem(SESS_KEY) || "";
// v5 reconnect/replay: the last snapshot revision this page rendered. Sent as `?rev=` on every
// (re)connect so the server replays exactly the frames we missed — no gap, no flicker. Kept in
// sessionStorage (keyed by the token base — and, under a daemon, the session id — so it can
// never target a different server or session) to survive a page reload too.
function revKeyFor(sid) { return "forge-rev:" + BASE + (sid ? ":" + sid : ""); }
let REV_KEY = revKeyFor(curSession);
let lastRev = Number(sessionStorage.getItem(REV_KEY) || 0) || 0;

function boot() {
  fetch(BASE + "/api/sessions", { cache: "no-store" })
    .then(r => { if (!r.ok) throw new Error("single"); return r.json(); })
    .then(rows => {
      daemon = true;
      $("btnSessions").hidden = false;
      initPush();
      renderSessionList(rows);
      if (curSession && rows.some(r => r.id === curSession)) {
        connect();
      } else {
        curSession = "";
        showSessions(true);
      }
      setInterval(pollSessions, 5000);
    })
    .catch(() => connect()); // the in-chat single-session server
}

function connect() {
  if (dead) return;
  if (daemon && !curSession) return; // nothing attached yet — pick from the list
  const scheme = location.protocol === "https:" ? "wss://" : "ws://";
  const sess = daemon && curSession ? "&session=" + encodeURIComponent(curSession) : "";
  ws = new WebSocket(scheme + location.host + BASE + "/ws?rev=" + lastRev + sess);
  ws.onopen = () => { retries = 0; $("conn").textContent = "● connected"; flushOfflineQueue(); };
  ws.onmessage = (e) => {
    let s; try { s = JSON.parse(e.data); } catch { return; }
    // Dedupe on revision: a frame can arrive both in the reconnect replay and via the live
    // stream (the server guarantees no GAP by overlapping the two, and we drop the overlap
    // here). A resync frame always applies — its revision doesn't extend our stream.
    if (!s.resync && !s.closed && s.revision && s.revision <= lastRev) return;
    if (s.revision) {
      lastRev = s.revision;
      try { sessionStorage.setItem(REV_KEY, String(lastRev)); } catch (e2) {}
    }
    render(s);
    if (s.closed) {
      if (daemon) {
        // The session was archived (or the driver stopped) — back to the list; the daemon
        // (and every other session) is still very much alive.
        detach();
        showSessions(true);
        pollSessions();
        $("conn").textContent = "session archived";
      } else {
        dead = true;
        $("conn").textContent = "remote control turned off — reconnect to the TUI";
        ws.close();
      }
    }
  };
  ws.onclose = () => {
    if (dead) return;
    retries++;
    // After ~12s of failures the session is almost certainly gone (the server dies with the
    // TUI) — say so instead of an infinite "reconnecting…", and back off to a slow retry.
    $("conn").textContent = retries > 8
      ? (daemon ? "daemon unreachable — is forge serve running?"
                : "session unreachable — reopen /remote from the TUI for a fresh link")
      : "reconnecting…";
    setTimeout(connect, retries > 8 ? 10000 : 1500);
  };
  ws.onerror = () => ws.close();
}

// --- v6 session control (forge serve) -------------------------------------------------------
function detach() {
  if (ws) { ws.onclose = null; try { ws.close(); } catch (e) {} ws = null; }
  curSession = "";
  try { sessionStorage.removeItem(SESS_KEY); } catch (e) {}
}

function attach(id) {
  if (ws) { ws.onclose = null; try { ws.close(); } catch (e) {} ws = null; }
  curSession = id;
  try { sessionStorage.setItem(SESS_KEY, id); } catch (e) {}
  REV_KEY = revKeyFor(id);
  lastRev = Number(sessionStorage.getItem(REV_KEY) || 0) || 0;
  retries = 0;
  oqDropped = 0;
  renderOfflineQueue();
  // The transcript belongs to the previous session — renderTranscript resets it when the new
  // session id arrives, but clear eagerly so nothing stale flashes.
  $("hist").innerHTML = ""; $("tail").innerHTML = ""; $("tail")._sig = "";
  histSession = null; histOldest = null; histDone = false;
  showSessions(false);
  $("conn").textContent = "connecting…";
  connect();
}

function pollSessions() {
  if (!daemon || $("sessions").hidden) return;
  fetch(BASE + "/api/sessions", { cache: "no-store" })
    .then(r => r.json()).then(renderSessionList).catch(() => {});
}

function renderSessionList(rows) {
  const el = $("slist");
  const sig = JSON.stringify(rows);
  if (el._sig === sig) return;
  el._sig = sig;
  el.innerHTML = "";
  if (!rows.length) {
    el.innerHTML = '<div class="empty">no sessions — create one below</div>';
  }
  rows.forEach(r => {
    const d = document.createElement("div");
    d.className = "sessrow" + (r.id === curSession ? " cur" : "");
    const main = document.createElement("button");
    main.className = "sessmain";
    main.innerHTML = '<span class="sdot' + (r.busy ? " busy" : "") + '"></span><b>' +
      esc(r.title || r.id.slice(0, 8)) + '</b><span class="sinfo">' +
      esc(baseName(r.cwd)) + (r.worktree ? " · ⎇ worktree" : "") +
      " · $" + (r.cost_usd || 0).toFixed(4) + " · " + age(r.last_activity) + '</span>';
    main.onclick = () => attach(r.id);
    const arch = document.createElement("button");
    arch.className = "sarch";
    arch.textContent = "archive";
    arch.onclick = () => {
      if (!confirm("Archive this session? It stops running (history is kept).")) return;
      fetch(BASE + "/api/sessions/" + encodeURIComponent(r.id) + "/archive", { method: "POST" })
        .then(() => { if (r.id === curSession) detach(); pollSessions(); })
        .catch(() => {});
    };
    d.appendChild(main); d.appendChild(arch);
    el.appendChild(d);
  });
}

function age(t) {
  if (!t) return "";
  const s = Math.max(0, Math.floor(Date.now() / 1000 - t));
  if (s < 60) return s + "s ago";
  if (s < 3600) return Math.floor(s / 60) + "m ago";
  if (s < 86400) return Math.floor(s / 3600) + "h ago";
  return Math.floor(s / 86400) + "d ago";
}

function showSessions(v) {
  $("sessions").hidden = !v;
  $("transcript").hidden = v;
  $("tasks").hidden = true;
  $("agents").hidden = true;
  if (v) pollSessions();
}

$("btnSessions").onclick = () => showSessions($("sessions").hidden);
$("snew").onclick = () => { $("snewform").hidden = !$("snewform").hidden; };
$("ncreate").onclick = () => {
  $("nerr").hidden = true;
  const body = {
    cwd: $("ncwd").value.trim() || null,
    worktree: $("nwt").checked,
    title: $("ntitle").value.trim() || null,
  };
  $("ncreate").disabled = true;
  fetch(BASE + "/api/sessions", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  })
    .then(r => r.json().then(j => ({ ok: r.ok, j })))
    .then(({ ok, j }) => {
      if (!ok) { $("nerr").textContent = j.error || "create failed"; $("nerr").hidden = false; return; }
      $("snewform").hidden = true;
      $("ncwd").value = ""; $("ntitle").value = ""; $("nwt").checked = false;
      attach(j.id);
    })
    .catch(e => { $("nerr").textContent = "create failed: " + e; $("nerr").hidden = false; })
    .finally(() => { $("ncreate").disabled = false; });
};
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
  $("stitle").textContent = s.title ? "· " + s.title : "";
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
  // A session change (/new, resume) invalidates the paginated history — it belongs to the
  // session it was fetched from.
  if (s.session_id && s.session_id !== histSession) {
    if (histSession !== null) { $("hist").innerHTML = ""; histOldest = null; histDone = false; }
    histSession = s.session_id;
  }
  // A true resync (unfillable gap) may have skipped turns: drop the accumulated history so a
  // scroll-up refetches it from the store — which has everything that happened meanwhile.
  if (s.resync && $("hist").childElementCount) {
    $("hist").innerHTML = ""; histOldest = null; histDone = false;
  }
  const panel = $("transcript");
  const tail = $("tail");
  // Exact content signature — a length-based check missed equal-length ring
  // rotations/replacements and left stale lines on screen. Only the live tail is rebuilt:
  // the paginated history above it (#hist) accumulates and must survive every snapshot.
  const body = (s.transcript || []).join("\n") + (s.streaming ? "\n" + s.streaming : "");
  if (tail._sig === body) return; // unchanged
  const nearBottom = panel.scrollHeight - panel.scrollTop - panel.clientHeight < 80;
  tail.innerHTML = "";
  (s.transcript || []).forEach(line => { const d = document.createElement("div"); d.textContent = line; tail.appendChild(d); });
  if (s.streaming) {
    // The streaming edge is the RAW reply text (unlike the pre-rendered tail lines), so it
    // gets the full markdown treatment live.
    const d = document.createElement("div"); d.className = "stream";
    d.appendChild(mdRender(s.streaming));
    tail.appendChild(d);
  }
  if (nearBottom) panel.scrollTop = panel.scrollHeight;
  tail._sig = body;
}

// --- v5 full scrollback: paginated persisted history above the live tail -------------------
// Scrolling to the top of the transcript fetches the next-older page from
// GET __BASE__/api/history?before=<oldest seq>&limit=N and PREPENDS it, preserving the scroll
// position. The live snapshot transcript stays a short tail; this is the real scrollback.
let histSession = null, histOldest = null, histLoading = false, histDone = false;
const HIST_PAGE = 60;

$("transcript").addEventListener("scroll", () => {
  if ($("transcript").scrollTop < 60) loadHistory();
});

function loadHistory() {
  if (histLoading || histDone || !histSession) return;
  histLoading = true;
  $("histload").hidden = false;
  const q = histOldest === null ? "?limit=" + HIST_PAGE : "?before=" + histOldest + "&limit=" + HIST_PAGE;
  fetch(BASE + "/api/history" + q, { cache: "no-store" })
    .then(res => { if (!res.ok) throw new Error("http " + res.status); return res.json(); })
    .then(rows => {
      if (!rows.length) { histDone = true; return; }
      histOldest = rows[rows.length - 1].seq; // rows are newest-first
      const panel = $("transcript"), hist = $("hist");
      const beforeH = panel.scrollHeight;
      const frag = document.createDocumentFragment();
      rows.slice().reverse().forEach(r => frag.appendChild(histRow(r))); // oldest→newest
      hist.insertBefore(frag, hist.firstChild);
      // Preserve what the reader was looking at across the prepend.
      panel.scrollTop += panel.scrollHeight - beforeH;
      if (rows.length < HIST_PAGE) histDone = true;
    })
    .catch(() => {})
    .finally(() => { histLoading = false; $("histload").hidden = true; });
}

function histRow(r) {
  const d = document.createElement("div");
  const note = r.visibility === "ui";
  d.className = "msg " + (note ? "note" : (r.role === "user" ? "user" : "forge"));
  const head = document.createElement("div");
  head.className = "mrole";
  head.textContent = (note ? "note" : (r.role === "user" ? "you" : "forge")) + (r.model ? " · " + r.model : "");
  d.appendChild(head);
  const body = document.createElement("div");
  body.className = "mbody";
  body.appendChild(mdRender(r.content));
  d.appendChild(body);
  return d;
}

// --- v5 rich transcript: a minimal, safe markdown renderer + syntax highlighter ------------
// DOM is built via createElement/textContent ONLY — transcript content is never fed to
// innerHTML, so it can't inject markup (the CSP is the second line of defense, not the first).
// Supported: #-headings, fenced code (highlighted + tap-to-copy), -/*/1. lists, paragraphs;
// inline `code`, **bold**, *italic*, and [links](…) rendered as their text (never live anchors).
function mdRender(src) {
  const frag = document.createDocumentFragment();
  const lines = String(src || "").split("\n");
  let i = 0, para = [];
  const flushPara = () => {
    if (!para.length) return;
    const p = document.createElement("p");
    p.appendChild(inlineMd(para.join("\n")));
    frag.appendChild(p);
    para = [];
  };
  while (i < lines.length) {
    const l = lines[i];
    const fence = l.match(/^\s*```([\w+-]*)\s*$/);
    if (fence) {
      flushPara();
      const code = [];
      i++;
      while (i < lines.length && !/^\s*```\s*$/.test(lines[i])) { code.push(lines[i]); i++; }
      i++; // past the closing fence (or EOF)
      frag.appendChild(codeBlock(code.join("\n"), fence[1].toLowerCase()));
      continue;
    }
    const h = l.match(/^(#{1,6})\s+(.*)$/);
    if (h) {
      flushPara();
      // h3..h6 — phone-sized headings, never a giant h1 inside a chat bubble.
      const el = document.createElement("h" + Math.min(h[1].length + 2, 6));
      el.appendChild(inlineMd(h[2]));
      frag.appendChild(el);
      i++; continue;
    }
    const li = l.match(/^\s*([-*]|\d+\.)\s+(.*)$/);
    if (li) {
      flushPara();
      const ordered = /^\d/.test(li[1]);
      const listEl = document.createElement(ordered ? "ol" : "ul");
      while (i < lines.length) {
        const m = lines[i].match(/^\s*([-*]|\d+\.)\s+(.*)$/);
        if (!m || /^\d/.test(m[1]) !== ordered) break; // a marker change starts a new list
        const item = document.createElement("li");
        item.appendChild(inlineMd(m[2]));
        listEl.appendChild(item);
        i++;
      }
      frag.appendChild(listEl);
      continue;
    }
    if (!l.trim()) { flushPara(); i++; continue; }
    para.push(l);
    i++;
  }
  flushPara();
  return frag;
}

// Inline markdown: `code`, **bold**, *italic*, [text](url) → text. Everything lands in the DOM
// as text nodes / textContent, so nothing in the source can become markup.
function inlineMd(text) {
  const frag = document.createDocumentFragment();
  const re = /(`([^`]+)`)|(\*\*([^*]+)\*\*)|(\*([^*\s][^*]*)\*)|(\[([^\]]+)\]\(([^)]+)\))/g;
  let last = 0, m;
  while ((m = re.exec(text))) {
    if (m.index > last) frag.appendChild(document.createTextNode(text.slice(last, m.index)));
    if (m[2] !== undefined) { const c = document.createElement("code"); c.textContent = m[2]; frag.appendChild(c); }
    else if (m[4] !== undefined) { const b = document.createElement("b"); b.textContent = m[4]; frag.appendChild(b); }
    else if (m[6] !== undefined) { const it = document.createElement("i"); it.textContent = m[6]; frag.appendChild(it); }
    else if (m[8] !== undefined) frag.appendChild(document.createTextNode(m[8])); // link → its text
    last = re.lastIndex;
  }
  if (last < text.length) frag.appendChild(document.createTextNode(text.slice(last)));
  return frag;
}

// A fenced block: highlighted <pre><code> + a tap-to-copy button (uses the DEVICE clipboard,
// like the /copy bar).
function codeBlock(code, lang) {
  const wrap = document.createElement("div");
  wrap.className = "codeblock";
  const btn = document.createElement("button");
  btn.className = "codecopy";
  btn.type = "button";
  btn.textContent = "copy";
  btn.addEventListener("click", () => {
    copyText(code);
    btn.textContent = "copied";
    setTimeout(() => { btn.textContent = "copy"; }, 1200);
  });
  const pre = document.createElement("pre");
  const codeEl = document.createElement("code");
  codeEl.appendChild(highlight(code, lang || ""));
  pre.appendChild(codeEl);
  wrap.appendChild(btn);
  wrap.appendChild(pre);
  return wrap;
}

// Keyword sets for the built-in highlighter (self-contained — no CDN can pass the CSP anyway).
const HL_ALIAS = { ts: "js", tsx: "js", jsx: "js", javascript: "js", typescript: "js",
  py: "python", python3: "python", rs: "rust", sh: "bash", shell: "bash", zsh: "bash",
  console: "bash", golang: "go", jsonc: "json" };
const HL_KW = {
  rust: "as async await break const continue crate dyn else enum extern false fn for if impl in let loop match mod move mut pub ref return self Self static struct super trait true type unsafe use where while",
  js: "async await break case catch class const continue default delete do else export extends false finally for from function if import in instanceof let new null of return static switch this throw true try typeof undefined var void while yield",
  python: "and as assert async await break class continue def del elif else except False finally for from global if import in is lambda None nonlocal not or pass raise return self True try while with yield",
  go: "break case chan const continue default defer else fallthrough false for func go goto if import interface map nil package range return select struct switch true type var",
  bash: "case do done echo elif else esac exit export fi for function if in local return set shift then until while",
  json: "false null true",
};

// Minimal single-pass tokenizer: strings, comments, numbers, keywords. Unknown languages pass
// through as plain text. Output is spans built with textContent — highlighter input is
// untrusted transcript content and must never reach innerHTML.
function highlight(code, lang) {
  const frag = document.createDocumentFragment();
  const L = HL_ALIAS[lang] || lang;
  const kw = HL_KW[L];
  if (!kw) { frag.appendChild(document.createTextNode(code)); return frag; }
  const kws = new Set(kw.split(" "));
  const lineComment = (L === "python" || L === "bash") ? "#" : (L === "json" ? null : "//");
  const blockComment = (L === "rust" || L === "js" || L === "go") ? ["/*", "*/"] : null;
  let i = 0, plain = "";
  const flush = () => { if (plain) { frag.appendChild(document.createTextNode(plain)); plain = ""; } };
  const span = (cls, text) => {
    flush();
    const el = document.createElement("span");
    el.className = cls;
    el.textContent = text;
    frag.appendChild(el);
  };
  while (i < code.length) {
    const c = code[i];
    if (lineComment && code.startsWith(lineComment, i)) {
      let j = code.indexOf("\n", i); if (j < 0) j = code.length;
      span("tok-c", code.slice(i, j)); i = j; continue;
    }
    if (blockComment && code.startsWith(blockComment[0], i)) {
      let j = code.indexOf(blockComment[1], i + 2);
      j = j < 0 ? code.length : j + 2;
      span("tok-c", code.slice(i, j)); i = j; continue;
    }
    if (c === '"' || c === "'" || c === "`") {
      let j = i + 1;
      while (j < code.length && code[j] !== c && code[j] !== "\n") { if (code[j] === "\\") j++; j++; }
      j = Math.min(j + 1, code.length);
      span("tok-s", code.slice(i, j)); i = j; continue;
    }
    if (/[0-9]/.test(c) && !/[A-Za-z0-9_]/.test(code[i - 1] || "")) {
      let j = i; while (j < code.length && /[0-9a-fA-FxXoObB._]/.test(code[j])) j++;
      span("tok-n", code.slice(i, j)); i = j; continue;
    }
    if (/[A-Za-z_]/.test(c)) {
      let j = i; while (j < code.length && /[A-Za-z0-9_]/.test(code[j])) j++;
      const w = code.slice(i, j);
      if (kws.has(w)) span("tok-k", w); else plain += w;
      i = j; continue;
    }
    plain += c; i++;
  }
  flush();
  return frag;
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

function submit() {
  const v = $("prompt").value;
  if (!v.trim()) return;
  if (ws && ws.readyState === 1) {
    send({ kind: "prompt", text: v });
  } else {
    // Offline: queue locally (per server + session, so it can never flush into a different
    // session) and deliver in order the moment the WS reconnects.
    queueOffline(v);
  }
  $("prompt").value = "";
}

// --- offline input queue --------------------------------------------------------------------
// Prompts typed while the WS is down land in localStorage (survives reloads and the PWA being
// killed), render as "queued (offline)" above the actions area, and flush IN ORDER on
// reconnect. Bounded: past OQ_CAP entries new input is dropped loudly, never silently.
const OQ_CAP = 20;
let oqDropped = 0;
function oqKey() { return "forge-oq:" + BASE + ":" + curSession; }
function oqLoad() { try { return JSON.parse(localStorage.getItem(oqKey()) || "[]") || []; } catch (e) { return []; } }
function oqSave(q) { try { localStorage.setItem(oqKey(), JSON.stringify(q)); } catch (e) {} }

function queueOffline(text) {
  const q = oqLoad();
  if (q.length >= OQ_CAP) { oqDropped++; }
  else q.push(text);
  oqSave(q);
  renderOfflineQueue();
}

function renderOfflineQueue() {
  const el = $("offq");
  const q = oqLoad();
  if (!q.length && !oqDropped) { el.hidden = true; el.innerHTML = ""; return; }
  el.hidden = false;
  el.innerHTML = q.map(t => "📴 queued (offline): " + esc(t)).join("<br>") +
    (oqDropped ? '<br><span class="oqfull">⚠ offline queue full (' + OQ_CAP + ") — " +
      oqDropped + " prompt" + (oqDropped > 1 ? "s" : "") + " dropped</span>" : "");
}

function flushOfflineQueue() {
  const q = oqLoad();
  if (q.length && ws && ws.readyState === 1) {
    q.forEach(t => send({ kind: "prompt", text: t }));
    oqSave([]);
  }
  oqDropped = 0;
  renderOfflineQueue();
}
renderOfflineQueue();
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
  $("sessions").hidden = true;
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

// --- Web push settings (forge serve only) ---------------------------------------------------
// Self-hosted VAPID push: notifications fire from YOUR daemon even with the page closed —
// permission prompts arrive with Allow/Deny actions the service worker answers directly.
// iOS caveat: Safari only exposes PushManager to an INSTALLED PWA (share → Add to Home Screen)
// on a trusted (or tunneled) HTTPS origin.
function b64ToU8(s) {
  const pad = "=".repeat((4 - (s.length % 4)) % 4);
  const raw = atob((s + pad).replace(/-/g, "+").replace(/_/g, "/"));
  return Uint8Array.from(raw, c => c.charCodeAt(0));
}

async function pushSubscription() {
  if (!("serviceWorker" in navigator) || !("PushManager" in window)) return null;
  const reg = await navigator.serviceWorker.ready;
  return reg.pushManager.getSubscription();
}

async function paintPush() {
  const btn = $("pushbtn"), info = $("pushinfo");
  if (!("serviceWorker" in navigator) || !("PushManager" in window)) {
    btn.hidden = true;
    info.textContent = "push is unsupported in this browser — on iPhone/iPad, install this " +
      "page to the home screen first (Share → Add to Home Screen), then enable push from the app.";
    return;
  }
  if (Notification.permission === "denied") {
    btn.hidden = true;
    info.textContent = "notification permission is blocked — allow notifications for this site " +
      "in the browser settings, then reload.";
    return;
  }
  const sub = await pushSubscription().catch(() => null);
  btn.hidden = false;
  btn.textContent = sub ? "Disable" : "Enable";
  info.textContent = sub
    ? "enabled — permission prompts, questions and finished turns notify this device even with " +
      "the page closed (answer Allow/Deny right from the notification)."
    : "off — enable to get notified (and approve prompts) with the page closed.";
}

async function enablePush() {
  const perm = await Notification.requestPermission();
  if (perm !== "granted") { paintPush(); return; }
  const r = await fetch(BASE + "/api/push/key", { cache: "no-store" });
  if (!r.ok) throw new Error("push key unavailable");
  const j = await r.json();
  const reg = await navigator.serviceWorker.ready;
  const sub = await reg.pushManager.subscribe({
    userVisibleOnly: true,
    applicationServerKey: b64ToU8(j.key),
  });
  const sj = sub.toJSON();
  const res = await fetch(BASE + "/api/push/subscribe", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ endpoint: sub.endpoint, keys: { p256dh: sj.keys.p256dh, auth: sj.keys.auth } }),
  });
  if (!res.ok) { try { await sub.unsubscribe(); } catch (e) {} throw new Error("subscribe failed"); }
}

async function disablePush() {
  const sub = await pushSubscription();
  if (!sub) return;
  const endpoint = sub.endpoint;
  try { await sub.unsubscribe(); } catch (e) {}
  await fetch(BASE + "/api/push/unsubscribe", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ endpoint }),
  }).catch(() => {});
}

function initPush() {
  $("pushrow").hidden = false;
  $("pushbtn").onclick = async () => {
    $("pushbtn").disabled = true;
    try {
      const sub = await pushSubscription();
      if (sub) await disablePush(); else await enablePush();
    } catch (e) {
      $("pushinfo").textContent = "push setup failed: " + (e && e.message || e);
    } finally {
      $("pushbtn").disabled = false;
      paintPush();
    }
  };
  paintPush();
}

// PWA: register the token-scoped service worker so the page installs to a home screen.
if ("serviceWorker" in navigator) {
  navigator.serviceWorker.register(BASE + "/sw.js", { scope: BASE + "/" }).catch(() => {});
}

$("prompt").focus();
boot();
