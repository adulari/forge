#!/usr/bin/env python3
"""CPU-profile one scenario through the WebKit ScriptProfiler and print top self-time functions.

    python3 profile.py PORT SCENARIO [--runs 6] [--top 25]

SCENARIO is a key of __bench.scenarios (tabTasksToChat, palette, ...) or `typing` / `sessionSwitch`.
Samples are aggregated by (function name, url:line); the web bundle is minified, so names are
only as good as the bundle -- export with `--no-minify` for readable ones.
"""
import argparse
import collections
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from inspector import Inspector  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("port")
    ap.add_argument("scenario")
    ap.add_argument("--runs", type=int, default=6)
    ap.add_argument("--top", type=int, default=25)
    a = ap.parse_args()
    ins = Inspector(a.port)
    ins.eval(open(os.path.join(HERE, "bench.js")).read())
    if a.scenario == "sessionsHover":
        ins.eval_async("__bench.sessions('hoverPrep')")
    if a.scenario == "sessionsRecent":
        ins.eval_async("__bench.sessions('warm')")
    ins.send("ScriptProfiler.startTracking", {"includeSamples": True})
    if a.scenario in ("typing",):
        ins.eval_async(f"__bench.typing({a.runs * 5})")
    elif a.scenario == "sessionsHover":
        ins.eval_async("__bench.sessions('hoverClick')")
    elif a.scenario == "sessionsRecent":
        ins.eval_async("__bench.sessions('recent')")
    elif a.scenario == "sessions":
        ins.eval_async("__bench.sessions()")
    else:
        ins.eval_async(f"__bench.runScenario('{a.scenario}', {a.runs})")
    ins.send("ScriptProfiler.stopTracking")
    selfc = collections.Counter()
    incl = collections.Counter()
    total = 0
    for ev in ins.events:
        if ev.get("method") not in ("ScriptProfiler.trackingUpdate", "ScriptProfiler.trackingComplete"):
            continue
        samples = ev["params"].get("samples") or {}
        for st in samples.get("stackTraces", []):
            frames = st.get("stackFrames", [])
            if not frames:
                continue
            total += 1
            def key(f):
                return f"{f.get('name') or '(anon)'} {os.path.basename(f.get('url') or '')}:{f.get('line')}"
            selfc[key(frames[0])] += 1
            for k in {key(f) for f in frames}:
                incl[k] += 1
    # the profiler reports samples at ~1ms intervals
    print(f"samples: {total}")
    print("-- top self time (samples) --")
    for k, v in selfc.most_common(a.top):
        print(f"{v:6} {k}")
    print("-- top inclusive --")
    for k, v in incl.most_common(a.top):
        print(f"{v:6} {k}")
    print("-- top inclusive, capitalised names (React components) --")
    comps = [(k, v) for k, v in incl.most_common() if k[:1].isupper() and ".js" in k]
    for k, v in comps[: a.top]:
        print(f"{v:6} {k}")


if __name__ == "__main__":
    main()
