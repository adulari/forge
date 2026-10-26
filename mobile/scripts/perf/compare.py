#!/usr/bin/env python3
"""A/B the interaction benchmark across two running instances, interleaved to cancel machine noise.

    python3 compare.py --a before=PORT --b after=PORT [--rounds 3] [--runs 10]
        [--restart-a 'cmd'] [--restart-b 'cmd']

--restart-* is a shell command run before each round (e.g. relaunch the instance with fresh
webview storage) so every round starts from the same cold state.

Each round runs desktop-interactions.py against A then B; the table reports, per scenario, the
median over rounds of each round's median (and the worst p95 seen), for both the paint-complete
latency and the commit latency (DOM final, before the frame is presented).
"""
import argparse
import json
import os
import statistics
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))


def run(port, runs, only):
    with tempfile.NamedTemporaryFile(suffix=".json", delete=False) as f:
        out = f.name
    cmd = [sys.executable, os.path.join(HERE, "desktop-interactions.py"), str(port), "--runs", str(runs), "--json", out]
    if only:
        cmd += ["--only", only]
    subprocess.run(cmd, check=True, stdout=subprocess.DEVNULL)
    return json.load(open(out))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--a", required=True)
    ap.add_argument("--b", required=True)
    ap.add_argument("--rounds", type=int, default=3)
    ap.add_argument("--runs", type=int, default=10)
    ap.add_argument("--only")
    ap.add_argument("--restart-a")
    ap.add_argument("--restart-b")
    ap.add_argument("--json")
    a = ap.parse_args()
    sides = [tuple(a.a.split("=")), tuple(a.b.split("="))]
    acc = {label: {} for label, _ in sides}
    for r in range(a.rounds):
        for (label, port), restart in zip(sides, (a.restart_a, a.restart_b)):
            if restart:
                subprocess.run(restart, shell=True, check=True, stdout=subprocess.DEVNULL)
            res = run(port, a.runs, a.only)
            for k, v in res.items():
                if v is None:
                    continue
                acc[label].setdefault(k, []).append(v)
            print(f"round {r + 1} {label} done", file=sys.stderr)

    def agg(label, k, field):
        vals = [x[field] for x in acc[label].get(k, []) if field in x]
        return statistics.median(vals) if vals else None

    def worst(label, k, field):
        vals = [x[field] for x in acc[label].get(k, []) if field in x]
        return max(vals) if vals else None

    la, lb = sides[0][0], sides[1][0]
    print(f"{'scenario':18} | {la+' paint':>14} {lb+' paint':>14} | {la+' commit':>15} {lb+' commit':>15} | worst p95 {la}/{lb}")
    summary = {}
    for k in acc[la]:
        if k not in acc[lb]:
            print(f"{k:18} | missing in {lb}")
            continue
        if "avgFrame" in acc[la][k][0]:
            fa, fb = agg(la, k, "avgFrame"), agg(lb, k, "avgFrame")
            pa, pb = agg(la, k, "p95Frame"), agg(lb, k, "p95Frame")
            print(f"{k:18} | frame avg {fa} -> {fb} ms, p95 frame {pa} -> {pb} ms")
            summary[k] = {"frameAvg": [fa, fb], "frameP95": [pa, pb]}
            continue
        row = {
            "paint": [agg(la, k, "med"), agg(lb, k, "med")],
            "commit": [agg(la, k, "commitMed"), agg(lb, k, "commitMed")],
            "p95": [worst(la, k, "p95"), worst(lb, k, "p95")],
        }
        summary[k] = row
        f = lambda v: "n/a" if v is None else v  # noqa: E731
        print(f"{k:18} | {f(row['paint'][0]):>14} {f(row['paint'][1]):>14} | {f(row['commit'][0]):>15} {f(row['commit'][1]):>15} | {f(row['p95'][0])}/{f(row['p95'][1])}")
    if a.json:
        json.dump(summary, open(a.json, "w"), indent=1)


if __name__ == "__main__":
    main()
