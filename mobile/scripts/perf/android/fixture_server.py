#!/usr/bin/env python3
"""Deterministic stand-in for `forge serve` with the same wire shape as the v10 daemon
(GET /api/sessions, GET /api/history, WS /ws snapshots), for measuring the mobile app on a device or
emulator without a real daemon. Needs the `websockets` package (>= 13).

  fixture_server.py --host 0.0.0.0 --port 7431 --token a1b2c3d4e5f60718293a4b5c6d7e8f90
  GET /<token>/bench/stream?session=<id>&tokens=600   start a ~50 tokens/s reply on that session

Session 3 of 24 (the one printed as `bench session:`) holds a 1,000-message markdown transcript.
Pair the app with `forge://connect?url=http%3A%2F%2F10.0.2.2%3A7431%2F<token>` (emulator), then
drive it with `bench_android.py`.
"""
import logging
import re
import argparse, asyncio, json, random, sys, time, uuid
from urllib.parse import urlparse, parse_qs
from websockets.asyncio.server import serve
from websockets.http11 import Response
from websockets.datastructures import Headers

NS = uuid.UUID("11111111-2222-3333-4444-555555555555")
TOPICS = ["flaky auth test", "migrate session store", "wire diff review", "fix streaming jank", "cache invalidation",
          "release notes", "lattice reindex", "clippy sweep", "onboarding flow", "push notifications", "mesh router",
          "voice capture", "settings screen", "anywhere pairing", "worktree cleanup", "hooks runner", "mcp client",
          "tui sidebar", "dist manifest", "android perf", "ios widgets", "daemon restart", "token gauge", "plan cards"]
CODE = {
 "rust": "fn reconcile(prev: &Snapshot, next: Snapshot) -> Decision {\n    if next.revision <= prev.revision && !next.resync {\n        return Decision::Drop;\n    }\n    let gap = next.revision.saturating_sub(prev.revision + 1);\n    if gap > 0 && !next.resync {\n        return Decision::Reconnect { missed: gap };\n    }\n    // A resync frame replaces everything, so accept the jump.\n    Decision::Accept(next)\n}\n\nimpl Decision {\n    fn is_terminal(&self) -> bool {\n        matches!(self, Decision::Reconnect { .. })\n    }\n}",
 "typescript": "export function accumulate(prev: Acc, tail: string, offset = 0): Acc {\n  if (offset <= 0) return { base: 0, text: tail };\n  const keep = offset - prev.base;\n  if (keep < 0 || keep > prev.text.length) {\n    return { base: offset, text: tail };\n  }\n  return { base: prev.base, text: prev.text.slice(0, keep) + tail };\n}\n\nconst ids = rows.map((r) => r.id).filter(Boolean);\nconsole.log(`loaded ${ids.length} rows`);",
 "python": "def page(rows, before=None, limit=200):\n    out = [r for r in rows if before is None or r['seq'] < before]\n    out.sort(key=lambda r: -r['seq'])\n    return out[:limit]\n\nfor r in page(rows, before=500, limit=3):\n    print(r['seq'], r['role'])",
}
WORDS = "the daemon streams full snapshots so the client keeps only the last revision and replays from there while the reply tail grows by a few tokens per frame and the transcript stays virtualized".split()

def sentence(rnd, n=14):
    w = [rnd.choice(WORDS) for _ in range(n)]
    w[0] = w[0].capitalize()
    return " ".join(w) + "."

def assistant_md(rnd, i, long=False):
    lang = rnd.choice(list(CODE))
    parts = [f"## Step {i}: {rnd.choice(TOPICS)}", "",
             f"{sentence(rnd)} **{rnd.choice(WORDS)}** and *{rnd.choice(WORDS)}* matter here, see `{rnd.choice(WORDS)}_{i}()` and [the docs](https://forge.adulari.dev/docs/{i}). {sentence(rnd, 10)}", "",
             f"- {sentence(rnd, 7)}", f"- {sentence(rnd, 9)} with `inline_code`", f"- {sentence(rnd, 6)}", "",
             f"```{lang}", CODE[lang], "```", "",
             f"> {sentence(rnd, 12)}", "", sentence(rnd, 20)]
    if long:
        parts += ["", "1. First follow up", "2. Second follow up", "", f"```{lang}", CODE[lang], "```"]
    return "\n".join(parts)

def build_messages(sid, count):
    rnd = random.Random(sid)
    rows = []
    seq = 0
    for i in range(count // 2):
        seq += 1
        rows.append({"seq": seq, "role": "user", "content": f"{sentence(rnd, 12)} Please take a look at {rnd.choice(TOPICS)}?", "model": None, "created_at": 1793900000 + seq, "visibility": "llm", "kind": "user", "elapsed_ms": 0, "tool": None})
        seq += 1
        rows.append({"seq": seq, "role": "assistant", "content": assistant_md(rnd, i), "model": "mock::mock", "created_at": 1793900000 + seq, "visibility": "llm", "kind": "assistant", "elapsed_ms": 0, "tool": None})
    return rows

class Session:
    def __init__(self, idx, bench):
        self.id = str(uuid.uuid5(NS, f"s{idx}"))
        self.idx = idx
        self.title = ("bench: " if bench else "") + TOPICS[idx % len(TOPICS)]
        self.msgs = build_messages(self.id, 1000 if bench else 40)
        self.clients = set()
        self.rev = 1
        self.busy = idx % 8 == 1
        self.waiting = idx % 8 == 0 and idx < 24
        self.starred = idx in (5, 13)
        self.done = idx in (22, 23)
        self.streaming = ""
        self.task = None

    def row(self):
        return {"id": self.id, "title": self.title, "cwd": f"/home/dev/{TOPICS[self.idx % len(TOPICS)].split()[0]}", "worktree": None,
                "busy": self.busy, "waiting": self.waiting, "last_turn_outcome": "success", "last_stop_reason": "final_answer",
                "cost_usd": round(0.01 * self.idx, 4), "context_tokens": 1200 * self.idx, "context_limit": 200000,
                "model": ["claude-cli::opus", "codex::gpt-5", "opencode::muse"][self.idx % 3], "permission_mode": "accept-edits",
                "created_at": 1793900000, "last_activity": 1793910000 - self.idx * 300, "read_only": False, "terminal": False,
                "dispatch_id": None, "dispatch_role": None, "dispatch_index": None, "provider": ["claude-cli", "codex", "opencode"][self.idx % 3],
                "parent_id": None, "diff_added": self.idx * 3 if self.idx % 2 else None, "diff_removed": self.idx if self.idx % 2 else None,
                "branch": f"forge/{TOPICS[self.idx % len(TOPICS)].replace(' ', '-')}", "starred": self.starred,
                "done_at": 1793909000 if self.done else None, "pr_watch": None, "duel_candidates": 0}

    def snapshot(self, resync=False):
        tail = self.streaming[-2048:]
        off = len(self.streaming) - len(tail)
        rows = [{"kind": m["kind"], "text": "  " + m["content"].split("\n")[0], "tool": None, "meta": None} for m in self.msgs[-6:]]
        return {"protocol": 10, "session_id": self.id, "title": self.title, "cwd": "/home/dev", "worktree": None,
                "project_initialized": True, "project_init_hint": None, "exposure": "daemon", "busy": self.busy, "done": False,
                "last_turn_outcome": None, "last_stop_reason": None, "turns_finished": 0, "temper": "Auto-edit",
                "permission_mode": "accept-edits", "effort": "medium", "tier": None, "model": "mock::mock", "model_pinned": False,
                "cost_usd": 0.0, "context_tokens": 0, "context_limit": None, "streaming": tail, "streaming_offset": off,
                "activity": "model reasoning" if self.busy else "", "turn_started_at": 0, "transcript": [r["text"] for r in rows],
                "transcript_rows": rows, "transcript_total": len(self.msgs), "tasks": [], "subagents": [], "queued": [],
                "permission_prompt": None, "question": None, "question_options": [], "question_allow_other": False,
                "question_index": 0, "overlay": None, "diff": None, "plan": None, "workflow": None, "suggested_prompt": None,
                "copy_text": None, "prompt_seq": 0, "notes": [], "revision": self.rev, "resync": resync, "closed": False}

    async def broadcast(self, resync=False):
        self.rev += 1
        data = json.dumps(self.snapshot(resync))
        for ws in list(self.clients):
            try:
                await ws.send(data)
            except Exception:
                self.clients.discard(ws)

    async def stream_reply(self, tokens, user_text=None):
        if self.task and not self.task.done():
            return
        async def run():
            if user_text:
                self.msgs.append({"seq": len(self.msgs) + 1, "role": "user", "content": user_text, "model": None, "created_at": int(time.time()), "visibility": "llm", "kind": "user", "elapsed_ms": 0, "tool": None})
            self.busy = True
            self.streaming = ""
            full = assistant_md(random.Random(self.rev), 9000, long=True)
            # ~4-char tokens, whitespace preserved so fences/lists stay valid markdown while streaming.
            one = re.findall(r"\S{1,4}|\s+", full + "\n\n")
            toks = (one * (tokens // len(one) + 1))[:tokens]
            await self.broadcast()
            t0 = time.monotonic()
            for n, tk in enumerate(toks):
                self.streaming += tk
                await self.broadcast()
                await asyncio.sleep(max(0, t0 + (n + 1) * 0.02 - time.monotonic()))
            self.msgs.append({"seq": len(self.msgs) + 1, "role": "assistant", "content": self.streaming.rstrip(), "model": "mock::mock", "created_at": int(time.time()), "visibility": "llm", "kind": "assistant", "elapsed_ms": 0, "tool": None})
            self.busy = False
            self.streaming = ""
            await self.broadcast()
        self.task = asyncio.create_task(run())

logging.getLogger("websockets").setLevel(logging.ERROR)

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=7431)
    ap.add_argument("--host", default="127.0.0.1")
    ap.add_argument("--token", default="a1b2c3d4e5f60718293a4b5c6d7e8f90")
    a = ap.parse_args()
    sessions = [Session(i, i == 2) for i in range(24)]
    by_id = {s.id: s for s in sessions}
    print("bench session:", sessions[2].id, flush=True)

    def http(status, body, ctype="application/json"):
        raw = body if isinstance(body, bytes) else json.dumps(body).encode()
        return Response(status, "OK" if status == 200 else "Not Found", Headers([("content-type", ctype), ("content-length", str(len(raw)))]), raw)

    async def process_request(conn, request):
        u = urlparse(request.path)
        q = {k: v[0] for k, v in parse_qs(u.query).items()}
        prefix = f"/{a.token}"
        if not u.path.startswith(prefix):
            return http(404, b"Not Found", "text/plain")
        path = u.path[len(prefix):]
        if path in ("/api/sessions", "/api/history", "/ws"):
            print(f"REQ {int(time.time()*1000)} {path}", flush=True)
        if path == "/ws":
            return None
        if path == "/api/sessions":
            return http(200, [s.row() for s in sessions])
        if path == "/api/history":
            s = by_id.get(q.get("session", ""))
            if not s:
                return http(200, [])
            rows = [m for m in s.msgs if "before" not in q or m["seq"] < int(q["before"])]
            if not q.get("include_tools"):
                rows = [m for m in rows if m["role"] != "tool"]
            rows.sort(key=lambda m: -m["seq"])
            return http(200, rows[: int(q.get("limit", 60))])
        if path == "/bench/stream":
            s = by_id[q["session"]]
            await s.stream_reply(int(q.get("tokens", 600)))
            return http(200, {"ok": True})
        print("unhandled", request.path, flush=True)
        return http(404, b"Not Found", "text/plain")

    async def handler(ws):
        u = urlparse(ws.request.path)
        q = {k: v[0] for k, v in parse_qs(u.query).items()}
        s = by_id.get(q.get("session", ""))
        if not s:
            await ws.close()
            return
        s.clients.add(ws)
        await ws.send(json.dumps(s.snapshot(resync=True)))
        try:
            async for raw in ws:
                m = json.loads(raw)
                if m.get("kind") == "prompt":
                    await s.stream_reply(300, user_text=m.get("text", ""))
        finally:
            s.clients.discard(ws)

    async def amain():
        async with serve(handler, a.host, a.port, process_request=process_request, ping_interval=None):
            print(f"fixture on http://{a.host}:{a.port}/{a.token}", flush=True)
            await asyncio.Future()
    asyncio.run(amain())

main()
