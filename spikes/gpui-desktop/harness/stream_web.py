#!/usr/bin/env python3
"""Baseline streaming: real daemon mock stream (FORGE_MOCK_STREAM_DELAY_MS=20 -> 50 words/s) into the open chat of the 1k-message session."""
import asyncio, json, os, sys, threading, time
sys.path.insert(0, os.path.expanduser("~/.cache/forge-spike-gpui"))
sys.path.insert(0, "/home/floris/Documents/Repositories/Personal/AI/forge-wt/spike-gpui/mobile/scripts/perf")
import mx, xin, measure_web as m
from inspector import Inspector
import websockets
H = m.H; ids = m.IDS
T = open(f"{H}/daemon/config/forge/serve-token").read().strip()
words = [0]; t_first = [None]; t_last = [None]
async def feed(n_prompts):
    ws = await websockets.connect(f"ws://127.0.0.1:7611/{T}/ws?session={ids['long2']}&rev=0", max_size=None)
    async def reader():
        async for msg in ws:
            try: j = json.loads(msg)
            except Exception: continue
            words[0] += 1
            if t_first[0] is None: t_first[0] = time.perf_counter()
            t_last[0] = time.perf_counter()
    r = asyncio.create_task(reader())
    for k in range(n_prompts):
        await ws.send(json.dumps({"kind": "prompt", "text": f"mock:code stream {k}"}))
        await asyncio.sleep(2.2)
    await asyncio.sleep(1.0); r.cancel(); await ws.close()
def main():
    pid = m.find_app(); xin.focus_largest(); i = Inspector(m.PORT)
    # open the stream-copy session in Chat mode (1,000 messages)
    xin.click(1165, 18); time.sleep(1.5)
    i.eval("history.pushState({}, '', '/session/%s');window.dispatchEvent(new PopStateEvent('popstate'));0" % ids["long2"]); time.sleep(4)
    i.eval(m.RAF); time.sleep(0.5)
    c0 = mx.cpu_s(pid); t0 = time.perf_counter()
    th = threading.Thread(target=lambda: asyncio.run(feed(6))); th.start(); th.join()
    wall = time.perf_counter() - t0; cpu = mx.cpu_s(pid) - c0
    ts = json.loads(i.eval("window.__go=false;JSON.stringify(window.__f)")); end = ts[-1]; ts = [t for t in ts if t >= end - wall * 1000]
    st = mx.frame_stats(ts); st.update(cpu_s=round(cpu, 2), wall_s=round(wall, 2), ws_frames=words[0], stream_window_s=round((t_last[0] - t_first[0]), 2) if t_first[0] else None)
    st["rss_mib"] = round(mx.rss_mib(pid), 0)
    print(json.dumps(st)); json.dump(st, open(f"{H}/results/web_stream.json", "w"), indent=1)
    import bl; bl.shot(i, f"{H}/results/web_stream.png")
main()
