#!/usr/bin/env python3
"""Independent audit of docs/benchmarks/claude-code-parity-2026-10.md against the raw bench-kit output.

    python3 audit_raw.py --kit ~/.cache/forge-ops/ccbench [--report PATH] [--commitments]

Standard library only. Deliberately shares no code with render.py and reads none of the derived JSON in this
directory: every number is recomputed from the kit's own files and compared with the table cell in the report.

  results/all-now/summary.json     suites a, b, d: the per-attempt rows (attempts) and the per-arm summaries (arms, pairs)
  results/c-agg/summary.json       suite c, one arm directory per task
  results/mt/<arm>/summary.json    multi-turn
  results/swe/<arm>/               official swebench report, predictions.metrics.jsonl, predictions.integrity.jsonl, summary.tsv

With --commitments it also recomputes the SHA-256 commitment of every task directory from suite/ and
compares it with the first 12 hex digits printed in the task tables. Exit status 1 on any mismatch.
"""
import argparse
import glob
import hashlib
import json
import math
import re
import statistics
import subprocess
import sys
from fractions import Fraction
from pathlib import Path

CHECKS = {"ok": 0, "bad": 0}
rep_path_holder = {"path": None}


def check(what, want, got):
    if want == got:
        CHECKS["ok"] += 1
    else:
        CHECKS["bad"] += 1
        print(f"MISMATCH {what}\n    report : {got!r}\n    raw    : {want!r}")


# ----------------------------------------------------------------------------- statistics
def sign_p(b, c):
    n = b + c
    if n == 0:
        return 1.0
    tail = sum(math.comb(n, i) for i in range(min(b, c) + 1))
    return float(min(Fraction(1), Fraction(2 * tail, 2 ** n)))


def wilson(k, n):
    z = 1.96
    p = k / n
    den = 1 + z * z / n
    mid = (p + z * z / (2 * n)) / den
    half = z * math.sqrt(p * (1 - p) / n + z * z / (4 * n * n)) / den
    return max(0.0, mid - half), min(1.0, mid + half)


def level(p):
    return "High" if p < 0.01 else "Moderate" if p < 0.05 else "Low"


def pv(p):
    return "p<0.001" if p < 0.001 else f"p={p:.3g}"


def med(xs):
    return statistics.median(xs)


# ----------------------------------------------------------------------------- report access
class Report:
    def __init__(self, path):
        self.text = Path(path).read_text(encoding="utf-8")

    def block(self, name):
        m = re.search(rf"<!-- BEGIN:{re.escape(name)} -->\n(.*?)\n<!-- END:{re.escape(name)} -->", self.text, re.S)
        if not m:
            raise SystemExit(f"audit_raw: report has no block {name}")
        return m.group(1)

    def tables(self, name):
        """All markdown tables of a block, as lists of rows (cells stripped of backticks)."""
        out, cur = [], []
        for line in self.block(name).splitlines():
            if line.startswith("|"):
                cells = [c.strip().replace("`", "") for c in re.split(r"(?<!\\)\|", line.strip())[1:-1]]
                if not all(re.fullmatch(r":?-{3}:?", c) for c in cells):
                    cur.append(cells)
            elif cur:
                out.append(cur)
                cur = []
        if cur:
            out.append(cur)
        return out

    def rows(self, name, table=0):
        t = self.tables(name)[table]
        return t[0], t[1:]

    def has(self, s):
        return " ".join(s.split()) in " ".join(self.text.split())


def i0(x):
    return f"{int(round(x)):,}"


def f1(x):
    return f"{x:,.1f}"


def pct(x):
    return f"{100 * x:.0f}%"


def xr(x):
    return f"x{x:.2f}"


def mi(n):
    return f"{n / 1e6:.2f}M" if n >= 1e6 else f"{n / 1e3:.0f}k" if n >= 1e4 else f"{n:,.0f}"


MODEL = {"deepseek": "DeepSeek V4.1 Flash", "haiku": "Haiku 5.5", "sonnet": "Sonnet 5.5"}


# ----------------------------------------------------------------------------- raw loading
def load_abd(kit):
    d = json.load(open(kit / "results/all-now/summary.json"))
    arms = {}
    for a in d["attempts"]:
        base, _, run = a["arm"].partition("@")
        seed = int(re.search(r"s(\d+)$", run).group(1)) if run else 1
        arms.setdefault(f"{base}/seed{seed}", {})[a["task"]] = a
    return d, arms


def load_c(kit):
    d = json.load(open(kit / "results/c-agg/summary.json"))
    arms = {}
    for a in d["attempts"]:
        base = a["arm"].partition("@")[0]
        arms.setdefault(f"{base}/seed1", {})[a["task"]] = a
    # an arm directory that produced no result row (FD c06: Forge panicked after the work was done) counts as a failed attempt
    for t in arms["CC-deepseek/seed1"]:
        arms["FD-deepseek/seed1"].setdefault(t, {"solved": False, "timed_out": False, "metrics_complete": False, "agent_wall": None,
                                                 "input_tokens": None, "output_tokens": None, "cached_input_tokens": None,
                                                 "tool_calls": None, "tool_errors": None, "model_calls": None})
    return d, arms


def tot(rows, key):
    v = [r[key] for r in rows if r.get(key) is not None]
    return sum(v) if v else None


def arm_numbers(rows):
    rows = list(rows)
    inp, cached = tot(rows, "input_tokens"), tot(rows, "cached_input_tokens") or 0
    return {
        "n": len(rows), "solved": sum(1 for r in rows if r["solved"]),
        "timeouts": sum(1 for r in rows if r["timed_out"]),
        "wall_med": med([r["agent_wall"] for r in rows if r["agent_wall"] is not None]),
        "wall_tot": tot(rows, "agent_wall"),
        "input": inp, "uncached": inp - cached, "output": tot(rows, "output_tokens"), "hit": cached / inp,
        "tools": tot(rows, "tool_calls"), "errors": tot(rows, "tool_errors"), "calls": tot(rows, "model_calls"),
        "incomplete": sum(1 for r in rows if not r["metrics_complete"]),
    }


def val(row, key):
    if key == "uncached":
        return None if row.get("input_tokens") is None else row["input_tokens"] - (row.get("cached_input_tokens") or 0)
    return row.get(key)


def paired(cc, fg, key, complete_only):
    ts = [t for t in cc if t in fg and val(cc[t], key) is not None and val(fg[t], key) is not None
          and not (complete_only and (not cc[t]["metrics_complete"] or not fg[t]["metrics_complete"]))]
    xs, ys = [val(cc[t], key) for t in ts], [val(fg[t], key) for t in ts]
    return {
        "n": len(ts), "cc": sum(xs), "fg": sum(ys), "ratio": sum(ys) / sum(xs),
        "fg_lower": sum(1 for x, y in zip(xs, ys) if y < x), "cc_lower": sum(1 for x, y in zip(xs, ys) if x < y),
        "med_ratio": med([y / x for x, y in zip(xs, ys) if x]),
    }


def quality_pair(cc, fg):
    ts = [t for t in cc if t in fg]
    return (sum(1 for t in ts if cc[t]["solved"] and not fg[t]["solved"]),
            sum(1 for t in ts if fg[t]["solved"] and not cc[t]["solved"]))


def verdict_label(a, b, p, higher_better):
    """(label, level, p): a = Claude Code value, b = Forge value, p = exact sign test."""
    if a == b:
        return "Equal", None, p
    lean = "Claude Code" if ((a > b) == higher_better) else "Forge"
    if p < 0.05:
        return lean, level(p), p
    return f"Within noise, {lean} ahead", "Low", p


def combine(vs):
    labels = [v[0] for v in vs]
    if len(vs) == 1:
        v = vs[0]
        return v[0], "n/a, identical" if v[0] == "Equal" else f"{v[1]}, {pv(v[2])}"
    lows = [v[1] for v in vs if v[1]]
    order = {"Low": 0, "Moderate": 1, "High": 2}
    lowest = min(lows, key=order.get) if lows else "n/a"
    ps = ", ".join(pv(v[2]) for v in vs)
    sides = {re.sub(r"Within noise, | ahead", "", l) for l in labels if l != "Equal"}
    if all(l.startswith("Within noise") or l == "Equal" for l in labels):
        if all(l == "Equal" for l in labels):
            lab = "Equal"
        elif len(sides) == 1 and "Equal" not in labels:
            lab = f"Within noise, {next(iter(sides))} ahead in both repeats"
        else:
            lab = "Within noise, direction differs by repeat" if len(sides) > 1 else "Within noise"
    elif len(set(labels)) == 1:
        lab = f"{labels[0]} (both repeats)"
    else:
        phrase = lambda l: "within noise" if l.startswith("Within noise") else l
        lab = ", ".join(f"{phrase(l)} in repeat {k + 1}" for k, l in enumerate(labels))
        lab = lab[0].upper() + lab[1:]
    return lab, f"{lowest} ({ps})"


# ----------------------------------------------------------------------------- audits
def audit_suite(rep, raw_doc, arms, tag, qs_block, eff_block, pair_blocks, matrix_block, kit_arm_summaries):
    """Per-arm and per-pair tables of one suite (abd or c)."""
    nums = {a: arm_numbers(rows.values()) for a, rows in arms.items()}
    _, qs = rep.rows(qs_block)
    for r in qs:
        a, n = r[0], nums[r[0]]
        check(f"{tag} {a} solved", f"{n['solved']}/{n['n']}", r[1])
        check(f"{tag} {a} pass rate", pct(n["solved"] / n["n"]), r[2])
        lo, hi = wilson(n["solved"], n["n"])
        check(f"{tag} {a} wilson", f"{100 * lo:.0f}-{100 * hi:.0f}%", r[3])
        check(f"{tag} {a} timeouts", str(n["timeouts"]), r[4])
        check(f"{tag} {a} wall median", f1(n["wall_med"]), r[5])
        check(f"{tag} {a} wall total", f1(n["wall_tot"]), r[6])
    _, eff = rep.rows(eff_block)
    for r in eff:
        a, n = r[0], nums[r[0]]
        want = [i0(n["input"]), i0(n["uncached"]), i0(n["output"]), pct(n["hit"]), i0(n["tools"]), i0(n["errors"]), i0(n["calls"]),
                str(n["incomplete"])]
        check(f"{tag} {a} efficiency row", want, r[1:])
    if kit_arm_summaries:
        for a, s in kit_arm_summaries.items():
            n = nums[a]
            for k_raw, k_n in (("input_tokens", "input"), ("output_tokens", "output"), ("tool_calls", "tools"),
                               ("model_calls", "calls"), ("tool_errors", "errors")):
                check(f"{tag} {a} kit-summary total {k_raw}", s["totals"][k_raw], n[k_n])
            check(f"{tag} {a} kit-summary solved", s["solved"], n["solved"])
            check(f"{tag} {a} kit-summary agent wall total", round(s["totals"]["agent_wall_secs"], 3), round(n["wall_tot"], 3))
    # pairs
    pairs = []
    for fid in arms:
        kind, model, seed = re.match(r"(CC|FB|FD)-(\w+)/seed(\d+)", fid).groups()
        if kind in ("FB", "FD") and f"CC-{model}/seed{seed}" in arms:
            pairs.append((f"CC-{model}/seed{seed}", fid, MODEL[model], f"{kind} vs CC", int(seed)))
    out = {}
    for cid, fid, mname, pname, seed in pairs:
        cc, fg = arms[cid], arms[fid]
        nc, nf = nums[cid], nums[fid]
        oc, of = quality_pair(cc, fg)
        wall = paired(cc, fg, "agent_wall", False)
        inp, outt = paired(cc, fg, "input_tokens", True), paired(cc, fg, "output_tokens", True)
        unc = paired(cc, fg, "uncached", True)
        tool, calls = paired(cc, fg, "tool_calls", True), paired(cc, fg, "model_calls", True)
        out[(mname, pname, seed)] = dict(nc=nc, nf=nf, oc=oc, of=of, p_q=sign_p(oc, of), wall=wall, inp=inp, unc=unc, outt=outt, tool=tool, calls=calls)
    # c-pairs has three tables in one block; abd has three blocks
    tabs = []
    for blk in pair_blocks:
        tabs += [rep.rows(blk, k) for k in range(len(rep.tables(blk)))]
    qual, speed, effr = tabs[0][1], tabs[1][1], tabs[2][1]
    for r in qual:
        k = (r[0], r[1], int(r[2]))
        x = out[k]
        want = [f"{x['nc']['solved']}/{x['nc']['n']}", f"{x['nf']['solved']}/{x['nf']['n']}",
                f"{100 * (x['nf']['solved'] / x['nf']['n'] - x['nc']['solved'] / x['nc']['n']):+.0f} pp", f"{x['oc']} / {x['of']}"]
        check(f"{tag} quality pair {k}", want, r[3:7])
        lab, lev, p = verdict_label(x["nc"]["solved"] / x["nc"]["n"], x["nf"]["solved"] / x["nf"]["n"], x["p_q"], True)
        check(f"{tag} quality verdict {k}", [lab, "n/a, identical" if lab == "Equal" else f"{lev}, {pv(p)}"], r[7:9])
    for r in speed:
        k = (r[0], r[1], int(r[2]))
        x = out[k]
        w = x["wall"]
        want = [f1(x["nc"]["wall_med"]), f1(x["nf"]["wall_med"]), xr(x["nf"]["wall_med"] / x["nc"]["wall_med"]), xr(w["med_ratio"]),
                f"{f1(x['nc']['wall_tot'])} / {f1(x['nf']['wall_tot'])}", f"{w['fg_lower']} of {w['n']}"]
        check(f"{tag} speed pair {k}", want, r[3:9])
        lab, lev, p = verdict_label(x["nc"]["wall_med"], x["nf"]["wall_med"], sign_p(w["fg_lower"], w["cc_lower"]), False)
        check(f"{tag} speed verdict {k}", [lab, f"{lev}, {pv(p)}"], r[9:11])
    for r in effr:
        k = (r[0], r[1], int(r[2]))
        x = out[k]
        want = [str(x["inp"]["n"]), xr(x["inp"]["ratio"]), xr(x["outt"]["ratio"]), xr(x["tool"]["ratio"]), xr(x["calls"]["ratio"]),
                f"{pct(x['nc']['hit'])} / {pct(x['nf']['hit'])}", xr(x["nf"]["uncached"] / x["nc"]["uncached"])]
        check(f"{tag} efficiency pair {k}", want, r[3:10])
    # matrix
    heads, rows = rep.rows(matrix_block)
    for r in rows:
        if r[0] == "**solved**":
            for h, c in zip(heads[1:], r[1:]):
                a = h.replace(" r", "/seed")
                check(f"{tag} matrix solved total {a}", str(nums[a]["solved"]), c)
            continue
        for h, c in zip(heads[1:], r[1:]):
            a = h.replace(" r", "/seed")
            row = arms[a].get(r[0])
            if row is None or row["agent_wall"] is None:
                check(f"{tag} matrix {a} {r[0]}", "F (no result)", c)
            else:
                check(f"{tag} matrix {a} {r[0]}", ("P " if row["solved"] else "F ") + f"{row['agent_wall']:.0f}", c)
    return nums, out


def audit_mt(rep, kit):
    mt = {a: json.load(open(kit / f"results/mt/{a}-deepseek/summary.json")) for a in ("FD", "CC")}
    t1_head, t1 = rep.rows("mt", 0)
    for r in t1:
        arm = r[0]
        d = mt[arm.split("-")[0]]
        runs = d["runs"]
        wall = sum(x["wall_secs"] for x in runs)
        want = [f"{sum(1 for x in runs if x['status'] == 'ok')}/{len(runs)}", pct(sum(1 for x in runs if x["grader_passed"]) / len(runs)),
                f"{statistics.mean(x['probe_score'] for x in runs):.2f}", f1(wall), i0(sum(x["input_tokens_total"] for x in runs)),
                i0(sum(x["input_tokens_total"] - x["cache_read_tokens"] for x in runs)), i0(sum(x["output_tokens"] for x in runs)),
                i0(sum(x["tool_calls"] for x in runs)), i0(sum(x["tool_errors"] for x in runs))]
        check(f"mt arm {arm}", want, r[1:])
        check(f"mt arm {arm} kit aggregate wall", round(d["aggregate"]["wall_secs_total"], 2), round(wall, 2))
        check(f"mt arm {arm} per-prompt walls add up to wall_secs",
              [round(sum(x["per_prompt_wall_secs"]), 2) for x in runs], [round(x["wall_secs"], 2) for x in runs])
    t2_head, t2 = rep.rows("mt", 1)
    for r in t2:
        run = next(x for x in mt[r[1].split("-")[0]]["runs"] if x["session"] == r[0])
        want = [run["prompts"], run["hidden_tests"], run["checks"], f"{run['probe_score']:.2f}", f1(run["wall_secs"]),
                i0(run["input_tokens_total"]), i0(run["input_tokens_total"] - run["cache_read_tokens"]), i0(run["output_tokens"]),
                str(run["tool_calls"]), str(run["tool_errors"])]
        check(f"mt session {r[0]} {r[1]}", want, r[2:])
    return mt


def load_swe(kit):
    out = {}
    for a in ("FD", "CC"):
        d = kit / f"results/swe/{a}-deepseek"
        rep_file = glob.glob(str(d / f"{a}-deepseek.eval_*.json"))
        assert len(rep_file) == 1, rep_file
        report = json.load(open(rep_file[0]))
        met = {m["instance_id"]: m for m in map(json.loads, filter(str.strip, open(d / "predictions.metrics.jsonl")))}
        integ = {m["instance_id"]: m for m in map(json.loads, filter(str.strip, open(d / "predictions.integrity.jsonl")))}
        lines = [ln.rstrip("\n").split("\t") for ln in open(d / "summary.tsv") if ln.strip()]
        tsv = {r[0]: dict(zip(lines[0], r)) for r in lines[1:]}
        out[f"{a}-deepseek"] = dict(report=report, met=met, integ=integ, tsv=tsv)
    return out


def audit_swe(rep, kit):
    swe = load_swe(kit)
    _, rows = rep.rows("swe", 0)
    for r in rows:
        s = swe[r[0]]
        m = list(s["met"].values())
        inp, cached = sum(x["input_tokens"] for x in m), sum(x.get("cached_input_tokens") or 0 for x in m)
        rid = set(s["report"]["resolved_ids"])
        want = [f"{len(rid)}/{len(m)}", f1(sum(x["wall_secs"] for x in m)), f1(med([x["wall_secs"] for x in m])), i0(inp), i0(inp - cached),
                i0(sum(x["output_tokens"] for x in m)),
                f"{sum(1 for x in s['integ'].values() if x['verdict'] == 'fail')} / {sum(1 for x in s['integ'].values() if x['verdict'] == 'warn')}",
                f"{sum(1 for x in s['integ'].values() if x['trace_checked'])}/{len(m)}",
                str(sum(1 for x in s["tsv"].values() if "refs/stash" in x["history"]))]
        check(f"swe arm {r[0]}", want, r[1:])
        check(f"swe arm {r[0]} report counts", [s["report"]["resolved_instances"], s["report"]["total_instances"], s["report"]["error_instances"]],
              [len(rid), len(m), 0])
        check(f"swe arm {r[0]} tsv vs metrics wall", {k: round(float(v["wall_s"]), 1) for k, v in s["tsv"].items()},
              {k: round(v["wall_secs"], 1) for k, v in s["met"].items()})
        check(f"swe arm {r[0]} tsv vs metrics tokens", {k: (int(v["in_tok"]), int(v["out_tok"])) for k, v in s["tsv"].items()},
              {k: (v["input_tokens"], v["output_tokens"]) for k, v in s["met"].items()})
    fd, cc = swe["FD-deepseek"], swe["CC-deepseek"]
    ids = sorted(fd["met"])
    rf, rc = set(fd["report"]["resolved_ids"]), set(cc["report"]["resolved_ids"])
    only_f, only_c = sum(1 for t in ids if t in rf and t not in rc), sum(1 for t in ids if t in rc and t not in rf)
    f_fast = sum(1 for t in ids if fd["met"][t]["wall_secs"] < cc["met"][t]["wall_secs"])
    c_fast = sum(1 for t in ids if cc["met"][t]["wall_secs"] < fd["met"][t]["wall_secs"])
    line = rep.block("swe").splitlines()[-1]
    check("swe paired line", (f"Paired over {len(ids)} instances: resolved only by FD-deepseek {only_f}, only by CC-deepseek {only_c} "
                              f"(sign test {pv(sign_p(only_f, only_c))}); FD-deepseek faster on {f_fast}, CC-deepseek faster on {c_fast} "
                              f"(sign test {pv(sign_p(f_fast, c_fast))})."), line)
    _, rows = rep.rows("swe-instances")
    for r in rows:
        t = r[0]
        want = []
        for a in ("FD-deepseek", "CC-deepseek"):
            s = swe[a]
            warn = any(f["rule"] == "test_path_touched" for f in s["integ"][t]["findings"])
            want += ["resolved" if t in s["report"]["resolved_ids"] else "unresolved", f1(s["met"][t]["wall_secs"]),
                     i0(s["met"][t]["output_tokens"]), "yes" if warn else "no"]
        check(f"swe instance {t}", want, r[1:])
    return swe


def audit_headline(rep, abd_out, c_out, mt, swe, arms_abd):
    """Recompute every row of the headline table."""
    want = {}

    def add(dim, sample, cc_txt, fg_txt, vs):
        lab, conf = combine(vs)
        want[(dim, sample)] = [cc_txt, fg_txt, lab, conf]

    groups = {}
    for (mname, pname, seed), x in abd_out.items():
        groups.setdefault((mname, pname), []).append((seed, x))

    def seeds(pr):
        return "repeats " + ", ".join(str(s) for s, _ in pr) if len(pr) > 1 else "1 repeat"

    for (mname, pname), pr in groups.items():
        smp = f"{mname}, {pname}, {seeds(pr)}, 26 tasks"
        add("Quality, suites a+b+d", smp, ", ".join(f"{x['nc']['solved']}/26" for _, x in pr), ", ".join(f"{x['nf']['solved']}/26" for _, x in pr),
            [verdict_label(x["nc"]["solved"], x["nf"]["solved"], x["p_q"], True) for _, x in pr])
    (cm_, cp_, _), cx_ = next(iter(c_out.items()))
    x = cx_
    add("Quality, suite c", f"{cm_}, {cp_}, 1 repeat, 11 tasks", f"{x['nc']['solved']}/{x['nc']['n']}", f"{x['nf']['solved']}/{x['nf']['n']}",
        [verdict_label(x["nc"]["solved"], x["nf"]["solved"], x["p_q"], True)])
    fdm, ccm = mt["FD"], mt["CC"]
    okf, okc = sum(1 for r in fdm["runs"] if r["grader_passed"]), sum(1 for r in ccm["runs"] if r["grader_passed"])
    add("Quality, multi-turn", "DeepSeek V4.1 Flash, FD vs CC, 1 repeat, 3 sessions", f"{okc}/3 graded pass", f"{okf}/3 graded pass",
        [verdict_label(okc / 3, okf / 3, 1.0, True)])
    fd, cc = swe["FD-deepseek"], swe["CC-deepseek"]
    ids = sorted(fd["met"])
    rf, rc = set(fd["report"]["resolved_ids"]), set(cc["report"]["resolved_ids"])
    of_, oc_ = sum(1 for t in ids if t in rf and t not in rc), sum(1 for t in ids if t in rc and t not in rf)
    add("Quality, SWE-bench Verified", "DeepSeek V4.1 Flash, FD vs CC, 1 attempt, 10 instances", f"{len(rc)}/10", f"{len(rf)}/10",
        [verdict_label(len(rc), len(rf), sign_p(oc_, of_), True)])
    for (mname, pname), pr in groups.items():
        smp = f"{mname}, {pname}, {seeds(pr)}, 26 tasks"
        add("Speed, suites a+b+d, median agent wall", smp, ", ".join(f"{x['nc']['wall_med']:.1f} s" for _, x in pr),
            ", ".join(f"{x['nf']['wall_med']:.1f} s" for _, x in pr),
            [verdict_label(x["nc"]["wall_med"], x["nf"]["wall_med"], sign_p(x["wall"]["fg_lower"], x["wall"]["cc_lower"]), False) for _, x in pr])
    add("Speed, suite c, median agent wall", f"{cm_}, {cp_}, 1 repeat, 11 tasks", f"{cx_['nc']['wall_med']:.1f} s", f"{cx_['nf']['wall_med']:.1f} s",
        [verdict_label(cx_["nc"]["wall_med"], cx_["nf"]["wall_med"], sign_p(cx_["wall"]["fg_lower"], cx_["wall"]["cc_lower"]), False)])
    ra, rb = ccm["runs"], fdm["runs"]
    fa, fb = sum(1 for x_, y in zip(ra, rb) if y["wall_secs"] < x_["wall_secs"]), sum(1 for x_, y in zip(ra, rb) if x_["wall_secs"] < y["wall_secs"])
    wa, wb = sum(r["wall_secs"] for r in ra), sum(r["wall_secs"] for r in rb)
    add("Speed, multi-turn, total wall", "DeepSeek V4.1 Flash, FD vs CC, 3 sessions", f"{wa:,.0f} s", f"{wb:,.0f} s",
        [verdict_label(wa, wb, sign_p(fa, fb), False)])
    fa, fb = (sum(1 for t in ids if fd["met"][t]["wall_secs"] < cc["met"][t]["wall_secs"]),
              sum(1 for t in ids if cc["met"][t]["wall_secs"] < fd["met"][t]["wall_secs"]))
    wa, wb = sum(cc["met"][t]["wall_secs"] for t in ids), sum(fd["met"][t]["wall_secs"] for t in ids)
    add("Speed, SWE-bench Verified, total wall", "DeepSeek V4.1 Flash, FD vs CC, 10 instances", f"{wa:,.0f} s", f"{wb:,.0f} s",
        [verdict_label(wa, wb, sign_p(fb, fa), False)])
    swe_fn = {"inp": lambda m: m["input_tokens"], "outt": lambda m: m["output_tokens"],
              "unc": lambda m: m["input_tokens"] - (m.get("cached_input_tokens") or 0)}
    for key, nice, mtkey in (("inp", "input tokens (cache reads included)", "input_tokens_total"),
                             ("unc", "input tokens not read from cache", "uncached_input_tokens"),
                             ("outt", "output tokens", "output_tokens")):
        for (mname, pname), pr in groups.items():
            smp = f"{mname}, {pname}, {seeds(pr)}, 26 tasks"
            add(f"Efficiency, suites a+b+d, {nice}", smp, ", ".join(mi(x[key]["cc"]) for _, x in pr), ", ".join(mi(x[key]["fg"]) for _, x in pr),
                [verdict_label(x[key]["cc"], x[key]["fg"], sign_p(x[key]["fg_lower"], x[key]["cc_lower"]), False) for _, x in pr])
        add(f"Efficiency, suite c, {nice}", f"{cm_}, {cp_}, 1 repeat, {cx_[key]['n']} tasks", mi(cx_[key]["cc"]), mi(cx_[key]["fg"]),
            [verdict_label(cx_[key]["cc"], cx_[key]["fg"], sign_p(cx_[key]["fg_lower"], cx_[key]["cc_lower"]), False)])
        fa = sum(1 for x_, y in zip(ra, rb) if y[mtkey] < x_[mtkey])
        fb = sum(1 for x_, y in zip(ra, rb) if x_[mtkey] < y[mtkey])
        ta, tb = sum(r[mtkey] for r in ra), sum(r[mtkey] for r in rb)
        add(f"Efficiency, multi-turn, {nice}", "DeepSeek V4.1 Flash, FD vs CC, 3 sessions", mi(ta), mi(tb), [verdict_label(ta, tb, sign_p(fa, fb), False)])
        va_, vb_ = {t: swe_fn[key](cc["met"][t]) for t in ids}, {t: swe_fn[key](fd["met"][t]) for t in ids}
        ta, tb = sum(va_.values()), sum(vb_.values())
        fa = sum(1 for t in ids if vb_[t] < va_[t])
        fb = sum(1 for t in ids if va_[t] < vb_[t])
        add(f"Efficiency, SWE-bench Verified, {nice}", "DeepSeek V4.1 Flash, FD vs CC, 10 instances", mi(ta), mi(tb),
            [verdict_label(ta, tb, sign_p(fa, fb), False)])
    _, rows = rep.rows("headline")
    seen = set()
    for r in rows:
        k = (r[0], r[1])
        seen.add(k)
        if k not in want:
            check(f"headline row {k} exists in raw recomputation", True, False)
            continue
        check(f"headline {k}", want[k], r[2:6])
    check("headline row count", len(want), len(rows))
    check("headline rows missing from report", set(), set(want) - seen)


def audit_prose(rep, abd_nums, abd_out, c_nums, c_out, swe, arms_abd, arms_c):
    """The numbers quoted in the running text. Each is computed here and must appear in the report verbatim."""
    def need(s, why):
        check(f"prose: {why}", True, rep.has(s))

    walls = [x["nf"]["wall_med"] / x["nc"]["wall_med"] for x in abd_out.values()]
    need(f"x{min(walls):.2f} to x{max(walls):.2f}", "median agent time range, a+b+d")
    cx = next(iter(c_out.values()))
    need(f"x{cx['nf']['wall_med'] / cx['nc']['wall_med']:.2f} on suite c", "suite c median ratio")
    fd, cc = swe["FD-deepseek"], swe["CC-deepseek"]
    ids = sorted(fd["met"])
    wr = sum(fd["met"][t]["wall_secs"] for t in ids) / sum(cc["met"][t]["wall_secs"] for t in ids)
    need(f"x{wr:.2f} in total on SWE-bench", "SWE-bench total time ratio")
    ds = {s: abd_out[("DeepSeek V4.1 Flash", "FD vs CC", s)] for s in (1, 2)}
    outs = [ds[1]["outt"]["ratio"], ds[2]["outt"]["ratio"], cx["outt"]["ratio"],
            sum(fd["met"][t]["output_tokens"] for t in ids) / sum(cc["met"][t]["output_tokens"] for t in ids)]
    need(f"x{min(outs):.1f} to x{max(outs):.1f} the output tokens", "output token range (FD vs CC)")
    tools = [ds[1]["tool"]["ratio"], ds[2]["tool"]["ratio"], cx["tool"]["ratio"]]
    need(f"x{min(tools):.1f} to x{max(tools):.1f} the tool calls", "tool call range (FD vs CC)")
    unc = [ds[1]["nf"]["uncached"] / ds[1]["nc"]["uncached"], ds[2]["nf"]["uncached"] / ds[2]["nc"]["uncached"],
           cx["nf"]["uncached"] / cx["nc"]["uncached"]]
    sw_unc = []
    for s in (fd, cc):
        m = list(s["met"].values())
        sw_unc.append(sum(x["input_tokens"] for x in m) - sum(x.get("cached_input_tokens") or 0 for x in m))
    unc.append(sw_unc[0] / sw_unc[1])
    need(f"x{min(unc):.1f} to x{max(unc):.0f} larger", "input-not-from-cache range")
    hits = {k: v["hit"] for k, v in abd_nums.items()}
    lo_, hi_ = sorted((hits["FD-deepseek/seed1"], hits["FD-deepseek/seed2"]))
    need(f"({100 * lo_:.0f}-{100 * hi_:.0f}% against {pct(hits['CC-deepseek/seed1'])} on a+b+d", "cache hit a+b+d")
    need(f"{pct(c_nums['FD-deepseek/seed1']['hit'])} against {pct(c_nums['CC-deepseek/seed1']['hit'])} on c", "cache hit c")
    fm, cm = list(fd["met"].values()), list(cc["met"].values())
    fh = sum(x.get("cached_input_tokens") or 0 for x in fm) / sum(x["input_tokens"] for x in fm)
    ch = sum(x.get("cached_input_tokens") or 0 for x in cm) / sum(x["input_tokens"] for x in cm)
    need(f"{pct(fh)} against {pct(ch)} on SWE-bench", "cache hit SWE")
    bridge = [abd_out[(m, "FB vs CC", s)] for m, s in (("Haiku 5.5", 1), ("Haiku 5.5", 2), ("Sonnet 5.5", 1))]
    ins = [b["inp"]["ratio"] for b in bridge]
    cs = [b["calls"]["ratio"] for b in bridge]
    need(f"{100 * (min(ins) - 1):.0f}% to {100 * (max(ins) - 1):.0f}% input tokens and {100 * (min(cs) - 1):.0f}% to {100 * (max(cs) - 1):.0f}% model calls",
         "bridge overhead range")
    # outcome-change counts between repeats, shared and unshared tasks
    flips = {}
    for key in ("CC-deepseek", "FD-deepseek", "CC-haiku", "FB-haiku"):
        a, b = arms_abd[f"{key}/seed1"], arms_abd[f"{key}/seed2"]
        flips[key] = sum(1 for t in a if a[t]["solved"] != b[t]["solved"])
    need("(" + ", ".join(f"{k} {v}" for k, v in flips.items()) + ")", "tasks that change outcome between repeats")
    need(f"{min(flips.values())} to {max(flips.values())} of the 26 tasks", "range of flips")
    tasks = list(arms_abd["CC-deepseek/seed1"])
    solved_by = {t: [a for a in arms_abd if arms_abd[a][t]["solved"]] for t in tasks}
    none = sorted(t for t in tasks if not solved_by[t])
    allten = [t for t in tasks if len(solved_by[t]) == len(arms_abd)]
    check("prose: tasks solved by no arm", ["a04", "a07", "a08", "a09"], [t.split("-")[0] for t in none])
    words = ["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve", "thirteen", "fourteen"]
    need(f"and {words[len(allten)]} by all ten arms", "tasks solved by all arms")
    need(f"only {words[len(tasks) - len(none) - len(allten)]} of the 26 tasks", "discriminating tasks")
    cc_arms = [a for a in arms_abd if a.startswith("CC-")]
    fg_arms = [a for a in arms_abd if not a.startswith("CC-")]
    check("prose: a01 solved by CC in all five CC arms", len(cc_arms), sum(1 for a in cc_arms if arms_abd[a]["a01-fix-back-to-back-conflicts"]["solved"]))
    check("prose: a01 solved by no Forge arm", 0, sum(1 for a in fg_arms if arms_abd[a]["a01-fix-back-to-back-conflicts"]["solved"]))
    check("prose: five CC arms", 5, len(cc_arms))
    check("prose: b09 solved by exactly FD seeds 1 and 2", ["FD-deepseek/seed1", "FD-deepseek/seed2"], sorted(solved_by["b09-fsweep-docs-drift"]))
    # reasoning tokens
    for s in (1, 2):
        a = arms_abd[f"FD-deepseek/seed{s}"]
        r = sum(x.get("reasoning_tokens") or 0 for x in a.values())
        o = sum(x["output_tokens"] for x in a.values())
        need(f"{i0(r)} of {i0(o)}", f"FD reasoning share, repeat {s}")
        need(f"{pct(r / o)}", "FD reasoning share percent")
    r = sum(x.get("reasoning_tokens") or 0 for x in fm)
    o = sum(x["output_tokens"] for x in fm)
    need(f"({i0(r)} of {i0(o)})", "FD SWE reasoning")
    need(f"{pct(r / o)} of its SWE-bench output", "FD SWE reasoning percent")
    # wilson widths
    widths = []
    for a, rows in arms_abd.items():
        n = arm_numbers(rows.values())
        lo, hi = wilson(n["solved"], n["n"])
        widths.append(round(100 * (hi - lo)))
    need(f"{min(widths)} to {max(widths)} points wide", "Wilson widths")
    # overlap: every pair of 26-task arms overlaps
    cis = [wilson(arm_numbers(rows.values())["solved"], 26) for rows in arms_abd.values()]
    check("prose: all ten 95% intervals overlap", True, all(a[0] <= b[1] and b[0] <= a[1] for a in cis for b in cis))
    # sonnet discordant
    s_ = abd_out[("Sonnet 5.5", "FB vs CC", 1)]
    check("prose: sonnet only-CC tasks", 3, s_["oc"])
    check("prose: sonnet only-FB tasks", 0, s_["of"])
    cd = next(iter(c_out.values()))
    check("prose: suite c discordant 4/0", (4, 0), (cd["oc"], cd["of"]))
    check("prose: suite c 'faster on 9 of 10'", 9, cd["wall"]["cc_lower"])
    need(f"x{cd['outt']['ratio']:.2f} the output", "suite c output ratio in reading")
    need(f"x{cd['tool']['ratio']:.2f} the tool calls", "suite c tool ratio in reading")
    # c tasks table
    _, rows = rep.rows("c-tasks")
    for r in rows:
        a, b = arms_c["CC-deepseek/seed1"][r[0]], arms_c["FD-deepseek/seed1"].get(r[0])
        check(f"c-tasks {r[0]} CC", ["P" if a["solved"] else "F", f1(a["agent_wall"]), i0(a["input_tokens"]), i0(a["output_tokens"]), i0(a["tool_calls"])],
              [r[1], r[3], r[5], r[7], r[9]])
        if b is None or b["agent_wall"] is None:
            check(f"c-tasks {r[0]} FD", ["F (no result)", "-", "-", "-", "-"], [r[2], r[4], r[6], r[8], r[10]])
        else:
            check(f"c-tasks {r[0]} FD", ["P" if b["solved"] else "F", f1(b["agent_wall"]), i0(b["input_tokens"]), i0(b["output_tokens"]), i0(b["tool_calls"])],
                  [r[2], r[4], r[6], r[8], r[10]])


def audit_logs(rep, kit):
    """The findings F1-F9 and the environment statements: counts taken from the raw run logs, streams and transcripts."""
    runs = kit / "results/runs"

    def need(s, why):
        check(f"prose: {why}", True, rep.has(s))

    def segments(path):
        txt = path.read_text(errors="replace")
        parts = re.split(r"^\[\d+/26\] (\S+)\n", txt, flags=re.M)
        return {parts[i].split("-")[0]: parts[i + 1] for i in range(1, len(parts), 2)}

    # F1: task-scope denials
    for run in ("base-s1", "base-s2"):
        seg = segments(runs / run / "FD-deepseek/seed1/run.log")
        check(f"F1 {run} FD denials only on a01, a04, a07", ["a01", "a04", "a07"],
              sorted(t for t, body in seg.items() if "permission denied by task scope" in body))
    c08 = (runs / "c/c08-gitignore-negation-parent-dir/FD-deepseek/seed1/run.log").read_text(errors="replace")
    check("F1 c08 FD denial present", True, "permission denied by task scope" in c08)
    others = [p.parts[-4] for p in sorted((runs / "c").glob("c*/FD-deepseek/seed1/run.log"))
              if "permission denied by task scope" in p.read_text(errors="replace")]
    check("F1 suite c denials only on c08", ["c08-gitignore-negation-parent-dir"], others)
    for run, arm in (("base-s1", "FB-haiku"), ("base-s2", "FB-haiku"), ("sonnet-s1", "FB-sonnet")):
        seg = segments(runs / run / arm / "seed1/run.log")
        ro = sorted(t for t in ("a01", "a04", "a07") if re.search(r"turn is read-only|session is read-only|session is marked read-only|scoped read-only|turn contract says not to change",
                                                                    seg.get(t, ""), re.I))
        check(f"F1 {run} {arm} says read-only on a01, a04, a07", ["a01", "a04", "a07"], ro)

    # F2: the c06 panic
    seed = json.load(open(runs / "c/c06-rename-config-provider-of/FD-deepseek/seed1/seed.json"))
    log = (runs / "c/c06-rename-config-provider-of/FD-deepseek/seed1/run.log").read_text(errors="replace")
    check("F2 exit code", 101, seed["exit_code"])
    check("F2 wall seconds", "165", f"{seed['wall_secs']:.0f}")
    check("F2 panic text", True, "crates/forge-core/src/lib.rs:3862:50" in log and "range start index 17 out of range for slice of length 10" in log)
    check("F2 agent summary says 28 files", True, "28 files, 71 lines swapped in place" in log)
    check("F2 no grader log", False, any((runs / "c/c06-rename-config-provider-of/FD-deepseek/seed1/tasks.logs").glob("*.test.log")))

    # F3: stash
    swe = kit / "results/swe"
    stash_logs = {}
    for cell in sorted((swe / "FD-deepseek/cells").iterdir()):
        t = (cell / "forge-bench.log").read_text(errors="replace")
        popped = any(l.lstrip().startswith("↳ shell(") and re.search(r"git (--no-pager )?stash pop", l) for l in t.splitlines())
        stash_logs[cell.name] = ("stashed the test edits and pushing back once" in t, popped)
    check("F3 warning in all ten FD logs", [True] * 10, [v[0] for v in stash_logs.values()])
    check("F3 both django logs run `git stash pop` as a shell call", [True, True],
          [stash_logs["django__django-13343"][1], stash_logs["django__django-13821"][1]])
    resets = [f for f in glob.glob(str(kit / "swe/.venv/lib/python*/site-packages/swebench/harness/test_spec/python.py"))
              if "git checkout {base_commit}" in open(f).read()]
    check("F3 swebench resets test files before the test patch", 1, len(resets))

    # F5: reasoning and the Claude Code counter
    streams = sorted((runs / "base-s1/CC-deepseek/seed1/streams").glob("*.jsonl"))
    think = text = tool = out = n_tool = n_think = 0
    for f in streams:
        res = None
        t_think = t_text = t_tool = 0
        for line in f.read_text(errors="replace").splitlines():
            try:
                o = json.loads(line)
            except ValueError:
                continue
            if o.get("type") == "assistant":
                for c in o["message"].get("content", []):
                    if c.get("type") == "thinking":
                        t_think += len(c.get("thinking", ""))
                        n_think += 1
                    elif c.get("type") == "text":
                        t_text += len(c.get("text", ""))
                    elif c.get("type") == "tool_use":
                        t_tool += len(json.dumps(c.get("input", {})))
                        n_tool += 1
            elif o.get("type") == "result":
                res = o.get("usage", {}).get("output_tokens")
        if res:
            think, text, tool, out = think + t_think, text + t_text, tool + t_tool, out + res
    check("F5 CC repeat 1: thinking blocks, tool calls", (261, 498), (n_think, n_tool))
    check("F5 CC reported output tokens (streams with a result)", 223861, out)
    check("F5 non-thinking estimate, thousands", 56, round((text + tool) / 4 / 1000))
    check("F5 with-thinking estimate, thousands", 189, round((think + text + tool) / 4 / 1000))
    d = json.load(open(kit / "results/all-now/summary.json"))
    ratios = [(a["reasoning_tokens"] or 0) / a["output_tokens"] for a in d["attempts"] if a["arm"].startswith("FD") and a["output_tokens"]]
    check("F5 FD reasoning share range", (30, 72), (round(100 * min(ratios)), round(100 * max(ratios))))
    for run in ("base-s1", "base-s2"):
        check(f"F5/F6 no compact_boundary in CC streams {run}", 0, sum("compact_boundary" in f.read_text(errors="replace")
                                                                       for f in (runs / run / "CC-deepseek/seed1/streams").glob("*.jsonl")))
    check("F6 no compact_boundary in suite c CC streams", 0, sum("compact_boundary" in f.read_text(errors="replace")
                                                                for f in (runs / "c").glob("c*/CC-deepseek/seed1/streams/*.jsonl")))

    # F6: compactions
    pat = re.compile(r"^\s*⟳ compacted \d+ → \d+ messages", re.M)
    per_c = {p.parts[-4].split("-")[0]: len(pat.findall(p.read_text(errors="replace"))) for p in sorted((runs / "c").glob("c*/FD-deepseek/seed1/run.log"))}
    check("F6 suite c compactions total", 30, sum(per_c.values()))
    check("F6 suite c compactions per task range", (1, 4), (min(per_c.values()), max(per_c.values())))
    check("F6 c02, c04, c10 have four each", ["c02", "c04", "c10"], sorted(t for t, n in per_c.items() if n == 4))
    check("F6 a+b+d compactions", (11, 14), tuple(len(pat.findall((runs / r / "FD-deepseek/seed1/run.log").read_text(errors="replace"))) for r in ("base-s1", "base-s2")))

    # F7: database is locked
    seg = segments(runs / "base-s2/FD-deepseek/seed1/run.log")
    lines = [l for l in seg["a05"].splitlines() if "database is locked" in l]
    check("F7 FD repeat 2 a05 lines", 8, len(lines))
    check("F7 FD repeat 2 session start failure", True, any("building session: starting session: database is locked" in l for l in lines))
    check("F7 FD repeat 1 none", 0, (runs / "base-s1/FD-deepseek/seed1/run.log").read_text(errors="replace").count("database is locked"))
    check("F7 FB-haiku lattice warnings", (1, 2), tuple(sum("database is locked" in l for l in (runs / r / "FB-haiku/seed1/run.log").read_text(errors="replace").splitlines()) for r in ("base-s1", "base-s2")))

    # F8: b09
    fails = {}
    for p in sorted(runs.glob("*/*/seed1/tasks.logs/b09-fsweep-docs-drift.attempt1.test.log")):
        if "sonnet" in p.parts[-5] and p.parts[-5] != "sonnet-s1":
            continue
        last = [l for l in p.read_text(errors="replace").splitlines() if l.startswith(("FAIL", "PASS"))]
        fails[f"{p.parts[-5]}/{p.parts[-4]}"] = last[-1] if last else ""
    passers = sorted(k.split("/")[1] for k, v in fails.items() if v == "PASS")
    check("F8 b09 passed only by FD-deepseek in both repeats", ["FD-deepseek", "FD-deepseek"], passers)
    helps = sum("--help output does not describe the dry-run output" in v for v in fails.values())
    verb = sum("docs/usage.md does not mention --verbose" in v for v in fails.values())
    check("F8 b09 failing lines among the eight other arms", (5, 3), (helps, verb))

    # F9: instruction files in the bridge transcripts and init events
    n = 0
    files = {}
    for run, arm in (("base-s1", "FB-haiku"), ("base-s2", "FB-haiku"), ("sonnet-s1", "FB-sonnet")):
        for f in (runs / run / arm / "seed1/transcripts").glob("*/*.jsonl"):
            n += 1
            seen = set()
            for line in f.read_text(errors="replace").splitlines():
                if '"instructions"' not in line:
                    continue
                try:
                    a = json.loads(line).get("attachment")
                except ValueError:
                    continue
                if isinstance(a, dict) and a.get("type") == "instructions":
                    seen |= {(x["path"], x["type"], len(x["content"])) for x in a["files"]}
            for x in seen:
                files[x] = files.get(x, 0) + 1
    check("F9 bridge transcripts", 78, n)
    check("F9 instruction files loaded as project instructions in every transcript",
          {("/home/floris/CLAUDE.md", "Project", 1434): 78, ("/home/floris/.claude/CLAUDE.md", "Project", 7049): 78}, files)
    init = {a["arm"]: a.get("claude_init") or {} for a in d["arms"]}
    check("F9 init skills/plugins, Anthropic-model arms", {(19, 3)}, {(init[k]["skills"], init[k]["plugins"]) for k in init if "haiku" in k and k.startswith("CC") or k == "CC-sonnet"})
    check("F9 init skills/plugins, DeepSeek CC arms", {(16, 2)}, {(init[k]["skills"], init[k]["plugins"]) for k in init if k.startswith("CC-deepseek")})
    check("F9 init mcp_servers empty", True, all(v.get("mcp_servers") == [] for v in init.values() if v))
    inv = json.loads((runs / "base-s1/FB-haiku/seed1/claude-invocations.jsonl").read_text().splitlines()[0])
    asp = inv["argv"][inv["argv"].index("--append-system-prompt") + 1]
    check("F4 bridge --append-system-prompt chars", 3025, int(re.search(r"<(\d+) chars", asp).group(1)))

    # multi-turn whole-session wall, SWE parallelism, instances
    mt = {a: json.load(open(kit / f"results/mt/{a}-deepseek/summary.json")) for a in ("FD", "CC")}
    ses = {a: sum(json.load(open(kit / f"results/mt/{a}-deepseek/seed1/{r['session']}.json"))["session_wall_secs"] for r in mt[a]["runs"]) for a in mt}
    need(f"{ses['FD']:.0f} s for Forge and {ses['CC']:.0f} s for Claude Code ({100 * (ses['FD'] / ses['CC'] - 1):.0f}% apart)", "multi-turn whole-session wall")
    for a in ("FD", "CC"):
        first = (kit / f"logs/swe-{a}-deepseek.log").read_text().splitlines()[0]
        check(f"SWE {a} ran two jobs", True, "jobs=2" in first)
    inst = {r["instance_id"]: r for r in map(json.loads, filter(str.strip, open(kit / "swe/instances.jsonl")))}
    _, rows = rep.rows("tasks-other")
    for r in rows:
        if r[1] == "SWE-bench Verified":
            x = inst[r[0]]
            check(f"SWE instance metadata {r[0]}", f"{x['repo']} {x['version']}, difficulty \"{x['difficulty']}\"", r[3])
    h = hashlib.sha256(open(kit / "swe/instances.jsonl", "rb").read()).hexdigest()
    check("SWE instances_sha256 prefix in the report", True, rep.has(f"`{h[:8]}...`"))
    check("SWE run-meta instances_sha256", h, json.load(open(kit / "results/swe/FD-deepseek/run-meta.json"))["instances_sha256"])

    # environment
    env = dict(r[:2] for r in rep.rows("env")[1])
    binary = kit / "bin/forge"
    check("env forge binary sha256", True, hashlib.sha256(binary.read_bytes()).hexdigest() in env["Forge binary"])
    check("env forge version", True, subprocess.run([str(binary), "--version"], capture_output=True, text=True).stdout.strip() == "forge 2.21.0" and env["Forge"].startswith("2.21.0,"))
    check("env claude version", subprocess.run(["claude", "--version"], capture_output=True, text=True).stdout.strip(), env["Claude Code"])
    check("env build log names the commit", True, "building 8b18cad8" in (kit.parent / "laptop-update.log").read_text())
    arm_meta = d["arm_meta"]
    check("env timeouts are 1200 s for suites a to d", {1200}, {m["timeout_secs"] for m in arm_meta.values()})
    par = (Path(rep_path_holder["path"]).parent / "artifacts/claude-code-parity-2026-10/baseline/PARITY.md").read_text()
    check("PARITY totals", True, "| **Total** | **40** | **110** | **71** | **59** | **280** |" in par)
    check("PARITY ranked gaps", 25, len(set(re.findall(r"^\| (G\d+) ", par, re.M))) or len(set(re.findall(r"\*\*(G\d+) ", par))))


def audit_commitments(rep, kit):
    root = kit / "suite"
    tar = ["tar", "--sort=name", "--mtime=@0", "--owner=0", "--group=0", "--numeric-owner", "-cf", "-"]
    _, rows = rep.rows("tasks")
    for r in rows:
        tid, suite = r[0], r[1]
        name = tid if suite == "c" else tid[1:]
        data = subprocess.run(tar + [name], cwd=root / suite, stdout=subprocess.PIPE, check=True).stdout
        check(f"commitment {tid}", hashlib.sha256(data).hexdigest()[:12], r[5])
    _, rows = rep.rows("tasks-other")
    for r in rows:
        if r[4] == "public":
            continue
        data = subprocess.run(tar + [r[0]], cwd=kit / "multiturn/sessions", stdout=subprocess.PIPE, check=True).stdout
        check(f"commitment {r[0]}", hashlib.sha256(data).hexdigest()[:12], r[4])


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--kit", required=True)
    ap.add_argument("--report", default=str(Path(__file__).resolve().parent.parent.parent / "claude-code-parity-2026-10.md"))
    ap.add_argument("--commitments", action="store_true")
    args = ap.parse_args()
    kit, rep = Path(args.kit).expanduser(), Report(args.report)
    rep_path_holder["path"] = args.report

    abd_doc, arms_abd = load_abd(kit)
    c_doc, arms_c = load_c(kit)
    # the kit's own per-arm summaries, as a second source for the totals
    abd_sums = {}
    for a in abd_doc["arms"]:
        base, _, run = a["arm"].partition("@")
        seed = int(re.search(r"s(\d+)$", run).group(1)) if run else 1
        abd_sums[f"{base}/seed{seed}"] = a
    abd_nums, abd_out = audit_suite(rep, abd_doc, arms_abd, "abd", "abd-arm-qs", "abd-arm-eff", ["abd-quality", "abd-speed", "abd-eff"],
                                    "matrix-abd", abd_sums)
    c_nums, c_out = audit_suite(rep, c_doc, arms_c, "c", "c-arm-qs", "c-arm-eff", ["c-pairs"], "matrix-c", None)
    mt = audit_mt(rep, kit)
    swe = audit_swe(rep, kit)
    audit_headline(rep, abd_out, c_out, mt, swe, arms_abd)
    audit_prose(rep, abd_nums, abd_out, c_nums, c_out, swe, arms_abd, arms_c)
    audit_logs(rep, kit)
    if args.commitments:
        audit_commitments(rep, kit)
    print(f"audit_raw: {CHECKS['ok']} checks agree with the raw files, {CHECKS['bad']} do not")
    return 1 if CHECKS["bad"] else 0


if __name__ == "__main__":
    sys.exit(main())
