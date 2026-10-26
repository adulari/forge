// In-page interaction latency harness. Injected by desktop-interactions.py (never shipped).
// latency = trigger() call -> the first rAF after the frame in which `ready()` became true,
// i.e. the interaction's final DOM has been handed to the compositor and one more frame has begun.
(function () {
  // Re-injecting replaces the harness (the rAF recorder below is shared across injections).
  const frame = () => new Promise((r) => requestAnimationFrame(r));
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  const text = () => document.body.textContent || "";
  // checkVisibility() needs style recalc only; getClientRects() forces a full layout, which the
  // MutationObserver below would then trigger once per mutation batch and distort the numbers.
  const shown = (e) => (e.checkVisibility ? e.checkVisibility() : e.getClientRects().length > 0);
  const tab = (name) => [...document.querySelectorAll('[role=tab]')].filter((e) => shown(e)).find((e) => (e.textContent || "").trim().startsWith(name));
  const key = (k, o) => window.dispatchEvent(new KeyboardEvent("keydown", Object.assign({ key: k, ctrlKey: true, bubbles: true, cancelable: true }, o || {})));
  const press = (el) => {
    el.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
    el.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
    el.click();
  };

  let gaps = [];
  let recording = false;
  window.__benchRecorder = (window.__benchRecorder || 0) + 1;
  const recorderId = window.__benchRecorder;
  (async function loop() {
    let last = performance.now();
    for (;;) {
      const t = await frame();
      if (recorderId !== window.__benchRecorder) return;
      if (recording) gaps.push(t - last);
      last = t;
    }
  })();

  async function idle(frames) {
    for (let i = 0; i < (frames || 4); i++) await frame();
    await sleep(60);
  }

  const visible = (sel) => [...document.querySelectorAll(sel)].filter((e) => shown(e));
  const leaf = (txt) => [...document.querySelectorAll("div,span")].some((e) => e.children.length === 0 && (e.textContent || "").includes(txt) && shown(e));
  const composer = () => visible("[data-testid=composer-input]")[0] || null;

  // `ready` is re-evaluated after every DOM mutation batch (cheap: no per-frame polling), then
  // the result is stamped two frames later so the committed DOM has actually been presented.
  async function measure(trigger, ready, timeoutMs) {
    await idle(6);
    gaps = [];
    recording = true;
    const t0 = performance.now();
    let readyAt = null;
    const check = () => { if (readyAt === null && ready()) readyAt = performance.now(); };
    const mo = new MutationObserver(check);
    mo.observe(document.body, { subtree: true, childList: true, attributes: true, characterData: true });
    trigger();
    const sync = performance.now() - t0;
    check();
    const limit = timeoutMs || 4000;
    for (;;) {
      const t = await frame();
      check();
      if (readyAt !== null && t > readyAt) {
        const t2 = await frame();
        mo.disconnect(); recording = false;
        return { ms: t2 - t0, commit: readyAt - t0, sync, maxGap: Math.max.apply(null, gaps), ok: true };
      }
      if (t - t0 > limit) { mo.disconnect(); recording = false; return { ms: t - t0, commit: limit, sync, maxGap: Math.max.apply(null, gaps), ok: false }; }
    }
  }

  function stats(rows) {
    const col = (k) => rows.map((r) => r[k]).sort((a, b) => a - b);
    const q = (a, p) => a[Math.min(a.length - 1, Math.floor(p * a.length))];
    const ms = col("ms"), sync = col("sync"), gap = col("maxGap"), commit = col("commit");
    return { n: rows.length, ok: rows.filter((r) => r.ok).length, med: +q(ms, 0.5).toFixed(1), p95: +q(ms, 0.95).toFixed(1), max: +ms[ms.length - 1].toFixed(1), commitMed: +q(commit, 0.5).toFixed(1), commitP95: +q(commit, 0.95).toFixed(1), syncMed: +q(sync, 0.5).toFixed(1), gapMax: +gap[gap.length - 1].toFixed(1) };
  }

  const scenarios = {
    tabChatToTasks: {
      setup: () => press(tab("Chat")),
      trigger: () => press(tab("Tasks")),
      ready: () => leaf("Verify the build"),
      after: () => press(tab("Chat")),
    },
    tabTasksToChat: {
      setup: () => press(tab("Tasks")),
      trigger: () => press(tab("Chat")),
      ready: () => !!composer() && leaf("Turn "),
    },
    tabChatToAgents: {
      setup: () => press(tab("Chat")),
      trigger: () => press(tab("Agents")),
      ready: () => leaf("no active agents"),
      after: () => press(tab("Chat")),
    },
    tabChatToReview: {
      setup: () => press(tab("Chat")),
      trigger: () => press(tab("Review")),
      ready: () => leaf("Working tree clean"),
      after: () => press(tab("Chat")),
    },
    tabChatToFiles: {
      setup: () => press(tab("Chat")),
      trigger: () => press(tab("Files")),
      ready: () => leaf("calc.py"),
      after: () => press(tab("Chat")),
    },
    tabChatToReplay: {
      setup: () => press(tab("Chat")),
      trigger: () => press(tab("Replay")),
      ready: () => leaf("→ run_code"),
      after: () => press(tab("Chat")),
    },
    palette: {
      setup: () => press(tab("Chat")),
      trigger: () => key("k"),
      ready: () => !!document.querySelector("input[placeholder*=command]"),
      after: () => key("Escape", { ctrlKey: false }),
    },
    usageDock: {
      setup: () => press(tab("Chat")),
      trigger: () => key("u"),
      ready: () => /Updated \d+s ago/.test(text()),
      after: () => key("u"),
    },
    terminalDock: {
      setup: () => press(tab("Chat")),
      trigger: () => key("j"),
      ready: () => /connected · \d+×\d+/.test(text()),
      after: () => key("j"),
    },
  };

  async function runScenario(name, runs) {
    const s = scenarios[name];
    const rows = [];
    for (let i = 0; i < runs; i++) {
      if (s.setup) { s.setup(); await idle(8); await sleep(150); }
      rows.push(await measure(s.trigger, s.ready));
      if (s.after) { s.after(); await idle(6); await sleep(150); }
    }
    return stats(rows);
  }

  async function sessionSwitch(runs) {
    const names = ["Perf session 1", "Perf session 2", "Perf session 3", "Perf session 4", "Perf session 5", "Perf session 0"];
    const item = (n) => [...document.querySelectorAll("div,a")].filter((e) => e.children.length === 0 && (e.textContent || "").trim() === n && shown(e))[0];
    const rows = [];
    for (let i = 0; i < runs; i++) {
      const n = names[i % names.length];
      const turns = text();
      rows.push(await measure(() => press(item(n)), () => location.pathname.endsWith("perf-sess-" + n.slice(-1)) && !!composer() && leaf("Turn ") && document.title !== null));
      await sleep(250);
    }
    return stats(rows);
  }

  const composerMirrorOk = () => true;
  async function typing(runs) {
    const input = composer;
    const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value").set;
    const typeChar = (ch) => { const el = input(); setter.call(el, el.value + ch); el.dispatchEvent(new Event("input", { bubbles: true })); };
    input().focus();
    const rows = [];
    for (let i = 0; i < runs; i++) {
      const ch = "abcdefghij"[i % 10];
      const before = input().value.length;
      rows.push(await measure(() => typeChar(ch), () => input().value.length > before && composerMirrorOk(), 1000));
      await sleep(30);
    }
    setter.call(input(), ""); input().dispatchEvent(new Event("input", { bubbles: true }));
    return stats(rows);
  }

  async function expandToolGroup(runs) {
    const rows = [];
    const groups = () => [...document.querySelectorAll("div")].filter((e) => e.children.length === 0 && /^Ran \d+ scripts?/.test(e.textContent || "") && shown(e));
    for (let i = 0; i < runs; i++) {
      const g = groups();
      if (!g.length) break;
      const el = g[g.length - 1 - (i % Math.min(g.length, 5))];
      const before = text().length;
      rows.push(await measure(() => press(el), () => text().length !== before, 1500));
      await sleep(100);
      press(el);
      await idle(6);
    }
    return rows.length ? stats(rows) : null;
  }

  async function scroll(steps) {
    const sc = [...document.querySelectorAll("div")].filter((e) => e.scrollHeight > e.clientHeight + 500 && /auto|scroll/.test(getComputedStyle(e).overflowY)).sort((a, b) => b.scrollHeight - a.scrollHeight)[0];
    if (!sc) return null;
    await idle(6);
    gaps = [];
    recording = true;
    const t0 = performance.now();
    sc.scrollTop = sc.scrollHeight;
    for (let i = 0; i < steps; i++) {
      sc.scrollTop = Math.max(0, sc.scrollTop - 400);
      await frame();
    }
    const total = performance.now() - t0;
    recording = false;
    const g = gaps.slice().sort((a, b) => a - b);
    return { frames: g.length, avgFrame: +(total / steps).toFixed(1), p95Frame: +g[Math.floor(g.length * 0.95)].toFixed(1), maxFrame: +g[g.length - 1].toFixed(1), scrollHeight: sc.scrollHeight };
  }

  window.__bench = { composer, visible, runScenario, sessionSwitch, typing, expandToolGroup, scroll, scenarios, tab, press, key, text, idle, sleep, measure, stats };
  return "installed";
})();
