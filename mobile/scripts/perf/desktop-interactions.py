#!/usr/bin/env python3
"""Interaction-latency benchmark for the desktop app, driven through the WebKit inspector.

Needs a running off-screen instance (see ~/.cache/forge-apptest/README.md) connected to a mock
daemon seeded with the synthetic `perf-sess-*` sessions (11 tasks, 70-turn transcripts).

    python3 desktop-interactions.py PORT [--runs 12] [--label after] [--json out.json]

Prints a table: median / p95 / max ms from input to the frame in which the new view is complete,
the synchronous part of the handler, and the worst rAF gap (a long-task proxy: WebKit has no
PerformanceObserver longtask).
"""
import argparse
import json
import os
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from inspector import Inspector  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("port")
    ap.add_argument("--runs", type=int, default=12)
    ap.add_argument("--label", default="run")
    ap.add_argument("--json")
    ap.add_argument("--only", help="comma separated scenario names")
    a = ap.parse_args()

    ins = Inspector(a.port)
    ins.eval(open(os.path.join(HERE, "bench.js")).read())
    ins.eval("(function(){var e=[...document.querySelectorAll('div,a')].filter(e=>e.children.length<4&&(e.innerText||'').trim().startsWith('Perf session 0')).pop();if(e){e.dispatchEvent(new MouseEvent('mousedown',{bubbles:true}));e.click();}return 0})()")
    for _ in range(100):
        if ins.eval("!!document.querySelector('[data-testid=composer-input]')"):
            break
        time.sleep(0.1)
    time.sleep(1.5)

    res = {}
    wanted = a.only.split(",") if a.only else None

    def want(n):
        return wanted is None or n in wanted

    for name in ["tabChatToTasks", "tabTasksToChat", "tabChatToAgents", "tabChatToReview", "tabChatToFiles", "tabChatToReplay", "palette", "usageDock", "terminalDock"]:
        if want(name):
            res[name] = ins.eval_async(f"__bench.runScenario('{name}', {a.runs})")
    if want("typing"):
        ins.eval_async("__bench.tab('Chat') && (__bench.press(__bench.tab('Chat')), __bench.idle(8))")
        res["typing"] = ins.eval_async(f"__bench.typing({max(a.runs, 30)})")
    if want("expandToolGroup"):
        res["expandToolGroup"] = ins.eval_async(f"__bench.expandToolGroup({a.runs})")
    if want("scroll"):
        res["scroll"] = ins.eval_async("__bench.scroll(60)")

    if want("sessionSwitch"):
        res["sessionSwitch"] = ins.eval_async(f"__bench.sessionSwitch({a.runs})")

    print(f"== {a.label} (runs={a.runs}) ==")
    print(f"{'scenario':20} {'paint-med':>9} {'paint-p95':>9} {'commit-med':>10} {'commit-p95':>10} {'sync':>6} {'gapMax':>7} ok")
    for k, v in res.items():
        if v is None:
            print(f"{k:20} n/a")
        elif "avgFrame" in v:
            print(f"{k:20} frame avg {v['avgFrame']} p95 {v['p95Frame']} max {v['maxFrame']} (scrollHeight {v['scrollHeight']})")
        else:
            print(f"{k:20} {v['med']:9} {v['p95']:9} {v['commitMed']:10} {v['commitP95']:10} {v['syncMed']:6} {v['gapMax']:7} {v['ok']}/{v['n']}")
    if a.json:
        json.dump(res, open(a.json, "w"), indent=1)


if __name__ == "__main__":
    main()
