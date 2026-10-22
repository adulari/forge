#!/usr/bin/env python3
"""Render the tables of docs/benchmarks/claude-code-parity-2026-10.md from the JSON in this directory.

    python3 render.py            rewrite every <!-- BEGIN:name --> ... <!-- END:name --> block in the report
    python3 render.py --check    exit 1 if the report on disk differs from what would be written
    python3 render.py --stdout   print all blocks (name + body) instead of touching the report
    python3 render.py --verify   recompute arm totals from the per-task rows and compare with the stored ones

Standard library only. Deterministic: the output depends on the JSON files and nothing else, so running it
twice is a no-op. runs.json lists the run sets. The first entry is the baseline; every later entry is an
"after" run set. The baseline tables are always rendered from the first entry. The after-* blocks compare the
LAST entry with the baseline (Forge arms only; Claude Code arms missing from the later run set are taken from
the baseline, because the harness under test is Forge).

Metric definitions (shared with runner/aggregate.py of the bench kit):
  agent wall       seconds from the end of setup.sh to the start of test.sh, measured by shims around the agent turn
  input tokens     uncached + cache reads + cache writes, as reported by the harness
  not from cache   input - cache reads (includes cache writes, which Anthropic bills above the plain input price)
  cache hit        cache reads / input tokens
  pass rate        solved / attempts; 95% interval is Wilson
  sign test p      exact two-sided binomial test on the discordant pairs (tasks only one arm solved), or on the
                   tasks where one arm was strictly cheaper/faster, ties dropped
"""
import json
import math
import re
import statistics
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPORT = HERE.parent.parent / "claude-code-parity-2026-10.md"
ALPHA = 0.05


# ----------------------------------------------------------------------------- loading
def load_json(rel):
    with open(HERE / rel, encoding="utf-8") as f:
        return json.load(f)


def runs_manifest():
    return load_json("runs.json")


class RunSet:
    def __init__(self, meta):
        self.meta = meta
        self.id = meta["id"]
        self.label = meta["label"]
        self.files = meta.get("files", {})

    def _get(self, key):
        rel = self.files.get(key)
        return load_json(rel) if rel else None

    def abd(self):
        return self._get("suites_abd")

    def suite_c(self):
        return self._get("suite_c")

    def tasks(self):
        return self._get("tasks")

    def environment(self):
        return self._get("environment")

    def multiturn(self):
        out = {}
        for arm, rel in (self.files.get("multiturn") or {}).items():
            out[arm] = load_json(rel)
        return out

    def swe(self):
        out = {}
        for arm, f in (self.files.get("swe") or {}).items():
            rec = {"report": load_json(f["report"]), "meta": load_json(f["run_meta"])}
            rec["metrics"] = load_jsonl(f["metrics"])
            rec["integrity"] = load_jsonl(f["integrity"])
            rec["tsv"] = load_tsv(f["summary_tsv"])
            out[arm] = rec
        return out


def load_jsonl(rel):
    rows = []
    with open(HERE / rel, encoding="utf-8") as f:
        for line in f:
            if line.strip():
                rows.append(json.loads(line))
    return rows


def load_tsv(rel):
    with open(HERE / rel, encoding="utf-8") as f:
        lines = [ln.rstrip("\n") for ln in f if ln.strip()]
    head = lines[0].split("\t")
    return [dict(zip(head, ln.split("\t"))) for ln in lines[1:]]


# ----------------------------------------------------------------------------- statistics
def wilson(k, n, z=1.96):
    if n == 0:
        return None
    p = k / n
    d = 1 + z * z / n
    c = p + z * z / (2 * n)
    h = z * math.sqrt(p * (1 - p) / n + z * z / (4 * n * n))
    return (max(0.0, (c - h) / d), min(1.0, (c + h) / d))


def sign_p(b, c):
    n = b + c
    if n == 0:
        return 1.0
    k = min(b, c)
    return min(1.0, 2 * sum(math.comb(n, i) for i in range(k + 1)) / 2 ** n)


def vals(rows, key):
    return [r[key] for r in rows if r.get(key) is not None]


def total(rows, key):
    v = vals(rows, key)
    return sum(v) if v else None


def median(rows, key):
    v = vals(rows, key)
    return statistics.median(v) if v else None


def arm_stats(arm):
    """Statistics of one arm, computed from its per-task rows. Totals and medians use every row whose field is
    not null, exactly like the bench kit's aggregate.py, so they match its summary."""
    rows = list(arm["tasks"].values())
    n = len(rows)
    solved = sum(1 for r in rows if r["solved"])
    inp, cached, cw = total(rows, "input_tokens"), total(rows, "cached_input_tokens"), total(rows, "cache_write_tokens")
    uncached = None
    if inp is not None:
        uncached = inp - (cached or 0)
    return {
        "n": n,
        "solved": solved,
        "pass_rate": solved / n if n else None,
        "wilson": wilson(solved, n),
        "timeouts": sum(1 for r in rows if r.get("timed_out")),
        "incomplete": sum(1 for r in rows if r.get("metrics_complete") is False),
        "agent_wall_total": total(rows, "agent_wall_s"),
        "agent_wall_median": median(rows, "agent_wall_s"),
        "wall_total_total": total(rows, "wall_total_s"),
        "wall_total_median": median(rows, "wall_total_s"),
        "input_tokens": inp,
        "output_tokens": total(rows, "output_tokens"),
        "cached_input_tokens": cached,
        "cache_write_tokens": cw,
        "uncached_input_tokens": uncached,
        "cache_hit": (cached / inp) if (inp and cached is not None) else None,
        "tool_calls": total(rows, "tool_calls"),
        "tool_errors": total(rows, "tool_errors"),
        "model_calls": total(rows, "model_calls"),
        "tool_calls_median": median(rows, "tool_calls"),
        "input_median": median(rows, "input_tokens"),
        "output_median": median(rows, "output_tokens"),
    }


def split_arm(arm_id):
    """'CC-deepseek/seed2' -> ('CC', 'deepseek', 2)."""
    m = re.match(r"^(CC|FB|FD)-([^/]+)/seed(\d+)$", arm_id)
    return (m.group(1), m.group(2), int(m.group(3))) if m else (None, None, None)


def cc_partner(arm_id):
    kind, model, seed = split_arm(arm_id)
    return f"CC-{model}/seed{seed}"


def forge_arm_ids(suite):
    return [a for a in suite["arms"] if split_arm(a)[0] in ("FB", "FD")]


def field(row, key):
    """A per-task value; 'uncached_input_tokens' is input minus cache reads, derived from the row."""
    if key == "uncached_input_tokens":
        if row.get("input_tokens") is None:
            return None
        return row["input_tokens"] - (row.get("cached_input_tokens") or 0)
    return row.get(key)


def pair_compare(cc, fg):
    """Paired comparison of a Forge arm against a Claude Code arm over the tasks both ran."""
    tasks = [t for t in cc["tasks"] if t in fg["tasks"]]
    only_cc = sum(1 for t in tasks if cc["tasks"][t]["solved"] and not fg["tasks"][t]["solved"])
    only_fg = sum(1 for t in tasks if fg["tasks"][t]["solved"] and not cc["tasks"][t]["solved"])
    out = {"n": len(tasks), "only_cc": only_cc, "only_fg": only_fg, "p_quality": sign_p(only_cc, only_fg)}

    def cmp_field(key, complete_only):
        ts = []
        for t in tasks:
            a, b = cc["tasks"][t], fg["tasks"][t]
            va, vb = field(a, key), field(b, key)
            if va is None or vb is None:
                continue
            if complete_only and (a.get("metrics_complete") is False or b.get("metrics_complete") is False):
                continue
            ts.append((va, vb))
        if not ts:
            return None
        sc, sf = sum(x for x, _ in ts), sum(y for _, y in ts)
        fg_lower = sum(1 for x, y in ts if y < x)
        cc_lower = sum(1 for x, y in ts if x < y)
        return {
            "n": len(ts), "cc_total": sc, "fg_total": sf,
            "ratio_total": (sf / sc) if sc else None,
            "forge_lower": fg_lower, "cc_lower": cc_lower, "p": sign_p(fg_lower, cc_lower),
            "ratio_median_paired": statistics.median([(y / x) for x, y in ts if x]) if any(x for x, _ in ts) else None,
        }

    out["agent_wall"] = cmp_field("agent_wall_s", False)
    out["input_tokens"] = cmp_field("input_tokens", True)
    out["uncached_input"] = cmp_field("uncached_input_tokens", True)
    out["output_tokens"] = cmp_field("output_tokens", True)
    out["tool_calls"] = cmp_field("tool_calls", True)
    out["model_calls"] = cmp_field("model_calls", True)
    return out


# ----------------------------------------------------------------------------- formatting
def i(n):
    return "n/a" if n is None else f"{int(round(n)):,}"


def f1(x):
    return "n/a" if x is None else f"{x:,.1f}"


def pct(x, d=0):
    return "n/a" if x is None else f"{100 * x:.{d}f}%"


def ci(w):
    return "n/a" if w is None else f"{100 * w[0]:.0f}-{100 * w[1]:.0f}%"


def xr(x):
    return "n/a" if x is None else f"x{x:.2f}"


def pv(p):
    """p value with its operator: 'p=0.25', 'p<0.001'."""
    return "p<0.001" if p < 0.001 else f"p={p:.3g}"


def mi(n):
    """Compact count: 6.73M, 224k."""
    if n is None:
        return "n/a"
    return f"{n / 1e6:.2f}M" if n >= 1e6 else f"{n / 1e3:.0f}k" if n >= 1e4 else f"{n:,.0f}"


def seed_of(arm_id):
    return split_arm(arm_id)[2]


def md(headers, rows, align=None):
    align = align or ["l"] + ["r"] * (len(headers) - 1)
    sep = {"l": "---", "r": "---:", "c": ":---:"}
    out = ["| " + " | ".join(headers) + " |", "|" + "|".join(sep[a] for a in align) + "|"]
    for r in rows:
        out.append("| " + " | ".join(str(c) for c in r) + " |")
    return "\n".join(out)


def esc(s):
    return str(s).replace("|", "\\|")


FAMILY_NAME = {"deepseek": "DeepSeek V4.1 Flash", "haiku": "Haiku 5.5", "sonnet": "Sonnet 5.5", "opus": "Opus"}


def model_name(m):
    return FAMILY_NAME.get(m, m)


LEVEL = {"Low": 0, "Moderate": 1, "High": 2}
NAME = {"cc": "Claude Code", "forge": "Forge"}


def verdict(a_val, b_val, p, higher_is_better):
    """Classify one comparison. a = Claude Code, b = Forge. Returns (class, lean, level, p): class is 'cc' or
    'forge' when the exact sign test reaches p < 0.05, 'tie' when it does not (lean keeps the direction of the
    point estimate), 'equal' for identical values, 'na' when a value is missing."""
    if a_val is None or b_val is None:
        return ("na", None, "n/a", None)
    if a_val == b_val:
        return ("equal", None, "n/a", p)
    lean = "cc" if ((a_val > b_val) == higher_is_better) else "forge"
    if p < 0.01:
        return (lean, lean, "High", p)
    if p < ALPHA:
        return (lean, lean, "Moderate", p)
    return ("tie", lean, "Low", p)


def label(v):
    cls, lean = v[0], v[1]
    if cls in ("cc", "forge"):
        return NAME[cls]
    if cls == "tie":
        return f"Within noise, {NAME[lean]} ahead"
    return {"equal": "Equal", "na": "n/a"}[cls]


def conf_text(v):
    if v[0] == "na":
        return "n/a"
    if v[0] == "equal":
        return "n/a, identical"
    return f"{v[2]}, {pv(v[3])}"


def combine(vs):
    """One label and confidence for several comparisons (one per seed)."""
    if len(vs) == 1:
        return label(vs[0]), conf_text(vs[0])
    classes = {v[0] for v in vs}
    leans = {v[1] for v in vs}
    if classes <= {"tie", "equal"}:
        if len(leans - {None}) == 1 and "equal" not in classes:
            lab = f"Within noise, {NAME[next(iter(leans))]} ahead in both repeats"
        elif len(leans - {None}) <= 1 and classes == {"equal"}:
            lab = "Equal"
        else:
            lab = "Within noise, direction differs by repeat" if len(leans - {None}) > 1 else "Within noise"
    elif len(classes) == 1:
        lab = f"{NAME[next(iter(classes))]} (both repeats)"
    else:
        phrase = {"cc": "Claude Code", "forge": "Forge", "tie": "within noise", "equal": "equal", "na": "n/a"}
        lab = ", ".join(f"{phrase[v[0]]} in repeat {k + 1}" for k, v in enumerate(vs))
        lab = lab[0].upper() + lab[1:]
    lowest = min((v[2] for v in vs if v[2] in LEVEL), key=lambda x: LEVEL[x], default="n/a")
    ps = ", ".join(pv(v[3]) for v in vs if v[3] is not None)
    return lab, f"{lowest} ({ps})"


# ----------------------------------------------------------------------------- blocks
def pairs_of(suite):
    out = []
    for fid in forge_arm_ids(suite):
        cid = cc_partner(fid)
        if cid in suite["arms"]:
            out.append((cid, fid))
    return out


def blk_env(rs):
    env = rs.environment()
    if not env:
        return "_No environment file._"
    rows = [
        ("Forge", f"{env['forge']['version']}, commit `{env['forge']['commit']}` ({env['forge']['commit_provenance']})"),
        ("Forge binary", f"`{env['forge']['binary']}`, SHA-256 `{env['forge']['binary_sha256']}`"),
        ("Claude Code", env["claude_code"]["version"]),
        ("Models actually used", "; ".join(f"`{m}`" for m in env["models_used"])),
        ("Provider for DeepSeek", env["deepseek_provider"]),
        ("SWE-bench evaluator", f"swebench {env['swebench']['version']} (official Docker harness), Docker {env['docker']}"),
        ("Python", env["python"]),
        ("CPU", f"{env['host']['cpu']}, {env['host']['cores']} threads"),
        ("RAM", env["host"]["ram"]),
        ("OS", f"{env['host']['os']}, kernel {env['host']['kernel']}"),
        ("Run window (UTC)", env["dates"]["window_utc"]),
        ("Concurrency", env["concurrency"]),
    ]
    return md(["Item", "Value"], [(a, esc(b)) for a, b in rows], ["l", "l"])


def blk_arms(rs):
    abd = rs.abd()
    seen, rows = set(), []
    for aid, arm in abd["arms"].items():
        kind, model, seed = split_arm(aid)
        key = f"{kind}-{model}"
        if key in seen:
            continue
        seen.add(key)
        seeds = sorted(seed_of(a) for a in abd["arms"] if a.startswith(key + "/"))
        init = arm.get("claude_init") or {}
        harness = {"CC": "Claude Code native loop", "FB": "Forge claude-cli bridge (Claude Code loop + Forge MCP tools and preamble)",
                   "FD": "Forge native loop"}[kind]
        rows.append((f"`{key}`", harness, f"`{arm['model']}`", f"`{init.get('model', arm['model_resolved'])}`",
                     ", ".join(str(s) for s in seeds)))
    return md(["Arm", "Harness", "Requested model", "Model id seen", "Repeats (suites a+b+d)"], rows, ["l", "l", "l", "l", "l"])


def blk_tasks(rs):
    t = rs.tasks()
    rows = []
    for x in t["tasks"]:
        rows.append((f"`{x['id']}`", x["suite"], x["language"], x["type"], esc(x["summary"]), f"`{x['sha256'][:12]}`"))
    return md(["Task", "Suite", "Language", "Type", "What is asked", "Commitment"], rows, ["l", "l", "l", "l", "l", "l"])


def blk_tasks_other(rs):
    t = rs.tasks()
    rows = []
    for x in t["multiturn"]:
        rows.append((f"`{x['id']}`", "multi-turn", x["language"], esc(x["summary"]), f"`{x['sha256'][:12]}`"))
    for x in t["swe"]["instances"]:
        rows.append((f"`{x['id']}`", "SWE-bench Verified", x["language"], esc(f"{x['repo']} {x['version']}, difficulty \"{x['difficulty']}\""), "public"))
    return md(["Id", "Dimension", "Language", "Notes", "Commitment"], rows, ["l", "l", "l", "l", "l"])


def per_arm_rows(suite, kind):
    rows = []
    for aid, arm in suite["arms"].items():
        s = arm_stats(arm)
        if kind == "qs":
            rows.append((f"`{aid}`", f"{s['solved']}/{s['n']}", pct(s["pass_rate"]), ci(s["wilson"]), s["timeouts"],
                         f1(s["agent_wall_median"]), f1(s["agent_wall_total"])))
        else:
            rows.append((f"`{aid}`", i(s["input_tokens"]), i(s["uncached_input_tokens"]), i(s["output_tokens"]),
                         pct(s["cache_hit"]), i(s["tool_calls"]), i(s["tool_errors"]), i(s["model_calls"]), s["incomplete"]))
    return rows


def blk_arm_qs(suite):
    return md(["Arm", "Solved", "Pass rate", "95% CI (Wilson)", "Timeouts", "Agent wall median (s)", "Agent wall total (s)"],
              per_arm_rows(suite, "qs"))


def blk_arm_eff(suite):
    return md(["Arm", "Input tokens", "Input not read from cache", "Output tokens", "Cache hit", "Tool calls", "Tool errors", "Model calls", "Rows with incomplete metrics"],
              per_arm_rows(suite, "eff"))


def incomplete_notes(suite):
    lines = []
    for aid, arm in suite["arms"].items():
        for t, r in arm["tasks"].items():
            if r.get("metrics_complete") is False or r.get("note"):
                lines.append(f"- `{aid}` `{t}`: {r.get('note') or ('no result row: ' + r['reason'] if r.get('reason') else 'metrics incomplete')}")
    return "\n".join(lines)


def blk_pair_quality(suite):
    rows = []
    for cid, fid in pairs_of(suite):
        cc, fg = suite["arms"][cid], suite["arms"][fid]
        sc, sf = arm_stats(cc), arm_stats(fg)
        pc = pair_compare(cc, fg)
        v = verdict(sc["pass_rate"], sf["pass_rate"], pc["p_quality"], True)
        kind, model, seed = split_arm(fid)
        rows.append((model_name(model), f"{kind} vs CC", seed, f"{sc['solved']}/{sc['n']}", f"{sf['solved']}/{sf['n']}",
                     f"{100 * (sf['pass_rate'] - sc['pass_rate']):+.0f} pp", f"{pc['only_cc']} / {pc['only_fg']}",
                     label(v), conf_text(v)))
    return md(["Model", "Pair", "Repeat", "Claude Code solved", "Forge solved", "Forge minus CC", "Solved only by CC / only by Forge",
               "Leads", "Confidence (sign test on the discordant tasks)"], rows, ["l", "l", "r", "r", "r", "r", "r", "l", "l"])


def blk_pair_speed(suite):
    rows = []
    for cid, fid in pairs_of(suite):
        cc, fg = suite["arms"][cid], suite["arms"][fid]
        sc, sf = arm_stats(cc), arm_stats(fg)
        pc = pair_compare(cc, fg)["agent_wall"]
        v = verdict(sc["agent_wall_median"], sf["agent_wall_median"], pc["p"], False)
        kind, model, seed = split_arm(fid)
        rows.append((model_name(model), f"{kind} vs CC", seed, f1(sc["agent_wall_median"]), f1(sf["agent_wall_median"]),
                     xr(sf["agent_wall_median"] / sc["agent_wall_median"]), xr(pc["ratio_median_paired"]),
                     f"{f1(sc['agent_wall_total'])} / {f1(sf['agent_wall_total'])}",
                     f"{pc['forge_lower']} of {pc['n']}", label(v), conf_text(v)))
    return md(["Model", "Pair", "Repeat", "CC median (s)", "Forge median (s)", "Ratio of medians", "Median paired ratio",
               "Total wall CC / Forge (s)", "Tasks where Forge was faster", "Leads", "Confidence (sign test over tasks)"],
              rows, ["l", "l", "r", "r", "r", "r", "r", "r", "r", "l", "l"])


def blk_pair_eff(suite):
    rows = []
    for cid, fid in pairs_of(suite):
        cc, fg = suite["arms"][cid], suite["arms"][fid]
        sc, sf = arm_stats(cc), arm_stats(fg)
        pc = pair_compare(cc, fg)
        kind, model, seed = split_arm(fid)
        unc_c, unc_f = sc["uncached_input_tokens"], sf["uncached_input_tokens"]
        rows.append((model_name(model), f"{kind} vs CC", seed, pc["input_tokens"]["n"],
                     xr(pc["input_tokens"]["ratio_total"]), xr(pc["output_tokens"]["ratio_total"]),
                     xr(pc["tool_calls"]["ratio_total"]) if pc["tool_calls"] else "n/a",
                     xr(pc["model_calls"]["ratio_total"]) if pc["model_calls"] else "n/a",
                     f"{pct(sc['cache_hit'])} / {pct(sf['cache_hit'])}",
                     xr(unc_f / unc_c) if unc_c else "n/a"))
    return md(["Model", "Pair", "Repeat", "Paired tasks with complete metrics", "Input tokens (Forge / CC)", "Output tokens (Forge / CC)",
               "Tool calls (Forge / CC)", "Model calls (Forge / CC)", "Cache hit CC / Forge", "Input not read from cache (Forge / CC, whole arm)"],
              rows, ["l", "l", "r", "r", "r", "r", "r", "r", "r", "r"])


def short_task(t):
    return t.split("-")[0]


def blk_matrix(suite, full_ids):
    arms = list(suite["arms"])
    tasks = list(next(iter(suite["arms"].values()))["tasks"])
    heads = ["Task"] + [a.replace("/seed", " r") for a in arms]
    rows = []
    for t in tasks:
        row = [f"`{t}`" if full_ids else f"`{short_task(t)}`"]
        for a in arms:
            r = suite["arms"][a]["tasks"].get(t)
            if r is None:
                row.append("-")
            elif r.get("outcome") == "no_result":
                row.append("F (no result)")
            else:
                w = r.get("agent_wall_s")
                row.append(("P " if r["solved"] else "F ") + (f"{w:.0f}" if w is not None else "n/a"))
        rows.append(row)
    solved = ["**solved**"] + [str(sum(1 for r in suite["arms"][a]["tasks"].values() if r["solved"])) for a in arms]
    rows.append(solved)
    return md(heads, rows, ["l"] + ["r"] * len(arms))


def blk_c_tasks(suite):
    ccid, fdid = "CC-deepseek/seed1", "FD-deepseek/seed1"
    cc, fd = suite["arms"][ccid]["tasks"], suite["arms"][fdid]["tasks"]
    rows = []
    for t in cc:
        a, b = cc[t], fd[t]

        def cell(r, key, fmt):
            v = r.get(key)
            return "-" if v is None else fmt(v)

        rows.append((f"`{t}`", "P" if a["solved"] else "F",
                     ("P" if b["solved"] else "F") + (" (no result)" if b.get("outcome") == "no_result" else ""),
                     cell(a, "agent_wall_s", f1), cell(b, "agent_wall_s", f1),
                     cell(a, "input_tokens", i), cell(b, "input_tokens", i),
                     cell(a, "output_tokens", i), cell(b, "output_tokens", i),
                     cell(a, "tool_calls", i), cell(b, "tool_calls", i),
                     f"{a['grader_score']}" if a.get("grader_score") else "-", f"{b['grader_score']}" if b.get("grader_score") else "-"))
    return md(["Task", "CC", "FD", "CC wall (s)", "FD wall (s)", "CC input", "FD input", "CC output", "FD output", "CC tools", "FD tools",
               "CC answer items", "FD answer items"], rows, ["l", "c", "c"] + ["r"] * 8 + ["r", "r"])


def blk_mt(mt):
    if not mt:
        return "_No multi-turn data._"
    rows, tot_rows = [], []
    arms = list(mt)
    sessions = [r["session"] for r in mt[arms[0]]["runs"]]
    for s in sessions:
        for a in arms:
            r = next(x for x in mt[a]["runs"] if x["session"] == s)
            rows.append((s, a, r["prompts"], r["hidden_tests"], r["checks"], f"{r['probe_score']:.2f}", f1(r["wall_secs"]),
                         i(r["input_tokens_total"]), i(r["uncached_input_tokens"]), i(r["output_tokens"]), r["tool_calls"], r["tool_errors"]))
    for a in arms:
        g = mt[a]["aggregate"]
        runs = mt[a]["runs"]
        tot_rows.append((a, f"{g['runs_ok']}/{g['runs']}", pct(g["grader_pass_rate"]), f"{g['probe_score_mean']:.2f}", f1(g["wall_secs_total"]),
                         i(g["input_tokens_total"]), i(sum(r["uncached_input_tokens"] for r in runs)), i(g["output_tokens_total"]),
                         i(g["tool_calls_total"]), i(sum(r["tool_errors"] for r in runs))))
    t1 = md(["Arm", "Sessions completed", "Hidden grader pass", "Probe score (mean)", "Wall total (s)", "Input tokens", "Input not read from cache",
             "Output tokens", "Tool calls", "Tool errors"], tot_rows)
    t2 = md(["Session", "Arm", "Prompts", "Hidden tests", "Checks", "Probe", "Wall (s)", "Input tokens", "Input not read from cache", "Output tokens",
             "Tool calls", "Tool errors"], rows, ["l", "l", "r", "r", "r", "r", "r", "r", "r", "r", "r", "r"])
    return t1 + "\n\n" + t2


def swe_rows(swe):
    out = {}
    for arm, rec in swe.items():
        resolved = set(rec["report"]["resolved_ids"])
        met = {m["instance_id"]: m for m in rec["metrics"]}
        integ = {m["instance_id"]: m for m in rec["integrity"]}
        hist = {m["instance_id"]: m for m in rec["tsv"]}
        out[arm] = {"resolved": resolved, "metrics": met, "integrity": integ, "history": hist,
                    "ids": [m["instance_id"] for m in rec["metrics"]]}
    return out


def blk_swe_summary(swe):
    if not swe:
        return "_No SWE-bench data._"
    d = swe_rows(swe)
    rows = []
    for arm, x in d.items():
        m = list(x["metrics"].values())
        n = len(m)
        res = len(x["resolved"])
        w = sum(r["wall_secs"] for r in m)
        inp, out_t = sum(r["input_tokens"] for r in m), sum(r["output_tokens"] for r in m)
        cached = sum(r.get("cached_input_tokens") or 0 for r in m)
        fails = sum(1 for r in x["integrity"].values() if r["verdict"] == "fail")
        warns = sum(1 for r in x["integrity"].values() if r["verdict"] == "warn")
        traced = sum(1 for r in x["integrity"].values() if r["trace_checked"])
        stash = sum(1 for r in x["history"].values() if "refs/stash" in r.get("history", ""))
        rows.append((arm, f"{res}/{n}", f1(w), f1(statistics.median([r["wall_secs"] for r in m])), i(inp), i(inp - cached), i(out_t),
                     f"{fails} / {warns}", f"{traced}/{n}", stash))
    out = md(["Arm", "Resolved (official)", "Wall total (s)", "Wall median (s)", "Input tokens", "Input not read from cache", "Output tokens",
              "Integrity fail / warn", "Traces audited", "Repos left with refs/stash"], rows)
    arms = list(d)
    if len(arms) == 2:
        a, b = d[arms[0]], d[arms[1]]
        ids = [t for t in a["ids"] if t in b["metrics"]]
        only_a = sum(1 for t in ids if t in a["resolved"] and t not in b["resolved"])
        only_b = sum(1 for t in ids if t in b["resolved"] and t not in a["resolved"])
        faster_a = sum(1 for t in ids if a["metrics"][t]["wall_secs"] < b["metrics"][t]["wall_secs"])
        faster_b = sum(1 for t in ids if b["metrics"][t]["wall_secs"] < a["metrics"][t]["wall_secs"])
        out += (f"\n\nPaired over {len(ids)} instances: resolved only by {arms[0]} {only_a}, only by {arms[1]} {only_b} "
                f"(sign test {pv(sign_p(only_a, only_b))}); {arms[0]} faster on {faster_a}, {arms[1]} faster on {faster_b} "
                f"(sign test {pv(sign_p(faster_a, faster_b))}).")
    return out


def blk_swe_instances(swe):
    if not swe:
        return "_No SWE-bench data._"
    d = swe_rows(swe)
    arms = list(d)
    ids = d[arms[0]]["ids"]
    heads = ["Instance"]
    for a in arms:
        heads += [f"{a} official", f"{a} wall (s)", f"{a} output tokens", f"{a} test files touched"]
    rows = []
    for t in ids:
        row = [f"`{t}`"]
        for a in arms:
            x = d[a]
            m = x["metrics"].get(t)
            iv = x["integrity"].get(t, {})
            warn = any(f["rule"] == "test_path_touched" for f in iv.get("findings", []))
            row += ["resolved" if t in x["resolved"] else "unresolved", f1(m["wall_secs"]), i(m["output_tokens"]), "yes" if warn else "no"]
        rows.append(row)
    return md(heads, rows, ["l"] + ["r", "r", "r", "r"] * len(arms))


def blk_ledger(rs_manifest):
    led = load_json(rs_manifest["ledger"])
    rows = []
    for p in led["prs"]:
        rows.append((f"#{p['number']}", esc(p["title"]), p["state"], esc(p["addresses"]), esc(p["status_of_evidence"])))
    return (md(["PR", "Title", f"State ({led['state_as']})", "Finding or gap it addresses", "Evidence it works"], rows, ["l", "l", "l", "l", "l"]))


# ----------------------------------------------------------------------------- headline
def headline(rs):
    abd, c, mt, swe = rs.abd(), rs.suite_c(), rs.multiturn(), rs.swe()
    rows = []

    def add(dim, sample, cc_txt, fg_txt, vs):
        lab, conf = combine(vs)
        rows.append((dim, sample, cc_txt, fg_txt, lab, conf))

    def group(suite):
        g = {}
        for cid, fid in pairs_of(suite):
            g.setdefault(split_arm(fid)[:2], []).append((suite["arms"][cid], suite["arms"][fid]))
        return g

    def seeds(pr):
        return "repeats " + ", ".join(str(a["seed"]) for a, _ in pr) if len(pr) > 1 else "1 repeat"

    def sample(kind, model, pr, n):
        return f"{model_name(model)}, {kind} vs CC, {seeds(pr)}, {n} tasks"

    gabd, gc = group(abd), group(c)
    d = swe_rows(swe) if len(swe) == 2 else None
    mtd = mt if len(mt) == 2 else None

    # quality
    for (kind, model), pr in gabd.items():
        vs = [verdict(arm_stats(a)["pass_rate"], arm_stats(b)["pass_rate"], pair_compare(a, b)["p_quality"], True) for a, b in pr]
        add("Quality, suites a+b+d", sample(kind, model, pr, 26),
            ", ".join(f"{arm_stats(a)['solved']}/26" for a, _ in pr), ", ".join(f"{arm_stats(b)['solved']}/26" for _, b in pr), vs)
    for (kind, model), pr in gc.items():
        a, b = pr[0]
        sa, sb = arm_stats(a), arm_stats(b)
        add("Quality, suite c", sample(kind, model, pr, 11), f"{sa['solved']}/{sa['n']}", f"{sb['solved']}/{sb['n']}",
            [verdict(sa["pass_rate"], sb["pass_rate"], pair_compare(a, b)["p_quality"], True)])
    if mtd:
        fd, cc_ = mtd["FD-deepseek"]["aggregate"], mtd["CC-deepseek"]["aggregate"]
        add("Quality, multi-turn", "DeepSeek V4.1 Flash, FD vs CC, 1 repeat, 3 sessions",
            f"{cc_['runs_ok']}/{cc_['runs']} graded pass", f"{fd['runs_ok']}/{fd['runs']} graded pass",
            [verdict(cc_["grader_pass_rate"], fd["grader_pass_rate"], 1.0, True)])
    if d:
        a, b = d["CC-deepseek"], d["FD-deepseek"]
        ids = a["ids"]
        oa = sum(1 for t in ids if t in a["resolved"] and t not in b["resolved"])
        ob = sum(1 for t in ids if t in b["resolved"] and t not in a["resolved"])
        add("Quality, SWE-bench Verified", "DeepSeek V4.1 Flash, FD vs CC, 1 attempt, 10 instances",
            f"{len(a['resolved'])}/{len(ids)}", f"{len(b['resolved'])}/{len(ids)}",
            [verdict(len(a["resolved"]), len(b["resolved"]), sign_p(oa, ob), True)])
    # speed
    for (kind, model), pr in gabd.items():
        vs = [verdict(arm_stats(a)["agent_wall_median"], arm_stats(b)["agent_wall_median"], pair_compare(a, b)["agent_wall"]["p"], False) for a, b in pr]
        add("Speed, suites a+b+d, median agent wall", sample(kind, model, pr, 26),
            ", ".join(f"{arm_stats(a)['agent_wall_median']:.1f} s" for a, _ in pr), ", ".join(f"{arm_stats(b)['agent_wall_median']:.1f} s" for _, b in pr), vs)
    for (kind, model), pr in gc.items():
        a, b = pr[0]
        sa, sb = arm_stats(a), arm_stats(b)
        add("Speed, suite c, median agent wall", sample(kind, model, pr, 11), f"{sa['agent_wall_median']:.1f} s", f"{sb['agent_wall_median']:.1f} s",
            [verdict(sa["agent_wall_median"], sb["agent_wall_median"], pair_compare(a, b)["agent_wall"]["p"], False)])
    if mtd:
        ra, rb = mtd["CC-deepseek"]["runs"], mtd["FD-deepseek"]["runs"]
        fa = sum(1 for x, y in zip(ra, rb) if y["wall_secs"] < x["wall_secs"])
        fb = sum(1 for x, y in zip(ra, rb) if x["wall_secs"] < y["wall_secs"])
        wa, wb = sum(x["wall_secs"] for x in ra), sum(y["wall_secs"] for y in rb)
        add("Speed, multi-turn, total wall", "DeepSeek V4.1 Flash, FD vs CC, 3 sessions", f"{wa:,.0f} s", f"{wb:,.0f} s", [verdict(wa, wb, sign_p(fa, fb), False)])
    if d:
        ids = a_ids = d["CC-deepseek"]["ids"]
        a, b = d["CC-deepseek"], d["FD-deepseek"]
        wa = sum(a["metrics"][t]["wall_secs"] for t in ids)
        wb = sum(b["metrics"][t]["wall_secs"] for t in ids)
        fa = sum(1 for t in ids if a["metrics"][t]["wall_secs"] < b["metrics"][t]["wall_secs"])
        fb = sum(1 for t in ids if b["metrics"][t]["wall_secs"] < a["metrics"][t]["wall_secs"])
        add("Speed, SWE-bench Verified, total wall", "DeepSeek V4.1 Flash, FD vs CC, 10 instances", f"{wa:,.0f} s", f"{wb:,.0f} s",
            [verdict(wa, wb, sign_p(fa, fb), False)])
    # efficiency: input (cache reads counted in full), input not read from cache, output
    for key, nice, mt_fld, swe_val in (
            ("input_tokens", "input tokens (cache reads included)", "input_tokens_total", lambda m: m["input_tokens"]),
            ("uncached_input", "input tokens not read from cache", "uncached_input_tokens",
             lambda m: m["input_tokens"] - (m.get("cached_input_tokens") or 0)),
            ("output_tokens", "output tokens", "output_tokens", lambda m: m["output_tokens"])):
        for (kind, model), pr in gabd.items():
            pcs = [pair_compare(a, b)[key] for a, b in pr]
            add(f"Efficiency, suites a+b+d, {nice}", sample(kind, model, pr, 26),
                ", ".join(mi(x["cc_total"]) for x in pcs), ", ".join(mi(x["fg_total"]) for x in pcs),
                [verdict(x["cc_total"], x["fg_total"], x["p"], False) for x in pcs])
        for (kind, model), pr in gc.items():
            x = pair_compare(*pr[0])[key]
            add(f"Efficiency, suite c, {nice}", sample(kind, model, pr, 10), mi(x["cc_total"]), mi(x["fg_total"]),
                [verdict(x["cc_total"], x["fg_total"], x["p"], False)])
        if mtd:
            ra, rb = mtd["CC-deepseek"]["runs"], mtd["FD-deepseek"]["runs"]
            fld = mt_fld
            fa = sum(1 for x, y in zip(ra, rb) if y[fld] < x[fld])
            fb = sum(1 for x, y in zip(ra, rb) if x[fld] < y[fld])
            ta, tb = sum(x[fld] for x in ra), sum(y[fld] for y in rb)
            add(f"Efficiency, multi-turn, {nice}", "DeepSeek V4.1 Flash, FD vs CC, 3 sessions", mi(ta), mi(tb), [verdict(ta, tb, sign_p(fa, fb), False)])
        if d:
            a, b = d["CC-deepseek"], d["FD-deepseek"]
            ids = a["ids"]
            va = {t: swe_val(a["metrics"][t]) for t in ids}
            vb = {t: swe_val(b["metrics"][t]) for t in ids}
            ta, tb = sum(va.values()), sum(vb.values())
            fa = sum(1 for t in ids if va[t] < vb[t])
            fb = sum(1 for t in ids if vb[t] < va[t])
            add(f"Efficiency, SWE-bench Verified, {nice}", "DeepSeek V4.1 Flash, FD vs CC, 10 instances", mi(ta), mi(tb), [verdict(ta, tb, sign_p(fa, fb), False)])
    return md(["Dimension", "Arms and sample", "Claude Code", "Forge", "Leads", "Confidence (lowest level over repeats; sign test)"], rows,
              ["l", "l", "r", "r", "l", "l"])


# ----------------------------------------------------------------------------- after fixes
def after_blocks(manifest, runs):
    """Side-by-side baseline vs the last run set, for the Forge arms present in it."""
    if len(runs) < 2:
        return None
    base, last = runs[0], runs[-1]
    out = {}
    for key, getter, name in (("after-abd", RunSet.abd, "suites a+b+d"), ("after-c", RunSet.suite_c, "suite c")):
        b, a = getter(base), getter(last)
        if not a:
            out[key] = f"_The {last.label} run set has no {name} data._"
            continue
        rows = []
        for fid in forge_arm_ids(a):
            cid = cc_partner(fid)
            cc = (a["arms"].get(cid) or b["arms"].get(cid))
            if cc is None:
                continue
            fa = a["arms"][fid]
            fb = b["arms"].get(fid)
            sa, sc = arm_stats(fa), arm_stats(cc)
            sb = arm_stats(fb) if fb else None
            pa = pair_compare(cc, fa)
            pb = pair_compare(cc, fb) if fb else None

            def pair(vb, va, fmt):
                return fmt(vb) if vb is not None else "n/a", fmt(va)

            rows.append((f"`{fid}`", f"{sc['solved']}/{sc['n']}",
                         f"{sb['solved']}/{sb['n']}" if sb else "n/a", f"{sa['solved']}/{sa['n']}",
                         f"{100 * (sa['pass_rate'] - sb['pass_rate']):+.0f} pp" if sb else "n/a",
                         f1(sc["agent_wall_median"]), f1(sb["agent_wall_median"]) if sb else "n/a", f1(sa["agent_wall_median"]),
                         xr(sa["agent_wall_median"] / sb["agent_wall_median"]) if sb else "n/a",
                         xr(pb["input_tokens"]["ratio_total"]) if pb else "n/a", xr(pa["input_tokens"]["ratio_total"]),
                         xr(pb["output_tokens"]["ratio_total"]) if pb else "n/a", xr(pa["output_tokens"]["ratio_total"])))
        out[key] = md(["Forge arm", "Claude Code solved", f"Baseline solved", f"{last.label} solved", "Delta", "CC wall median (s)",
                       "Baseline wall median (s)", f"{last.label} wall median (s)", "After / baseline wall",
                       "Baseline input (Forge / CC)", "After input (Forge / CC)", "Baseline output (Forge / CC)", "After output (Forge / CC)"],
                      rows, ["l"] + ["r"] * 12)
    mt_a, mt_b = last.multiturn(), base.multiturn()
    if mt_a:
        rows = []
        for arm, d in mt_a.items():
            g, gb = d["aggregate"], (mt_b.get(arm) or {}).get("aggregate")
            rows.append((arm, f"{gb['runs_ok']}/{gb['runs']}" if gb else "n/a", f"{g['runs_ok']}/{g['runs']}",
                         f1(gb["wall_secs_total"]) if gb else "n/a", f1(g["wall_secs_total"]),
                         i(gb["input_tokens_total"]) if gb else "n/a", i(g["input_tokens_total"])))
        out["after-mt"] = md(["Arm", "Baseline sessions ok", "After sessions ok", "Baseline wall (s)", "After wall (s)",
                              "Baseline input tokens", "After input tokens"], rows)
    else:
        out["after-mt"] = f"_The {last.label} run set has no multi-turn data._"
    swe_a, swe_b = last.swe(), base.swe()
    if swe_a:
        da, db = swe_rows(swe_a), swe_rows(swe_b)
        rows = []
        for arm, x in da.items():
            y = db.get(arm)
            n = len(x["ids"])
            rows.append((arm, f"{len(y['resolved'])}/{len(y['ids'])}" if y else "n/a", f"{len(x['resolved'])}/{n}",
                         f1(sum(m["wall_secs"] for m in y["metrics"].values())) if y else "n/a",
                         f1(sum(m["wall_secs"] for m in x["metrics"].values())),
                         i(sum(m["output_tokens"] for m in y["metrics"].values())) if y else "n/a",
                         i(sum(m["output_tokens"] for m in x["metrics"].values()))))
        out["after-swe"] = md(["Arm", "Baseline resolved", "After resolved", "Baseline wall (s)", "After wall (s)",
                               "Baseline output tokens", "After output tokens"], rows)
    else:
        out["after-swe"] = f"_The {last.label} run set has no SWE-bench data._"
    return out


# ----------------------------------------------------------------------------- driver
def build_blocks():
    manifest = runs_manifest()
    runs = [RunSet(m) for m in manifest["runs"]]
    base = runs[0]
    abd, c = base.abd(), base.suite_c()
    blocks = {
        "headline": headline(base),
        "env": blk_env(base),
        "arms": blk_arms(base),
        "tasks": blk_tasks(base),
        "tasks-other": blk_tasks_other(base),
        "abd-arm-qs": blk_arm_qs(abd),
        "abd-arm-eff": blk_arm_eff(abd),
        "abd-notes": incomplete_notes(abd),
        "abd-quality": blk_pair_quality(abd),
        "abd-speed": blk_pair_speed(abd),
        "abd-eff": blk_pair_eff(abd),
        "c-arm-qs": blk_arm_qs(c),
        "c-arm-eff": blk_arm_eff(c),
        "c-notes": incomplete_notes(c),
        "c-pairs": blk_pair_quality(c) + "\n\n" + blk_pair_speed(c) + "\n\n" + blk_pair_eff(c),
        "c-tasks": blk_c_tasks(c),
        "mt": blk_mt(base.multiturn()),
        "swe": blk_swe_summary(base.swe()),
        "swe-instances": blk_swe_instances(base.swe()),
        "matrix-abd": blk_matrix(abd, True),
        "matrix-c": blk_matrix(c, True),
        "ledger": blk_ledger(manifest),
    }
    after = after_blocks(manifest, runs)
    placeholder = "_No after-fixes run set has been recorded yet. Append one to `runs.json` and run `render.py`._"
    for k in ("after-abd", "after-c", "after-mt", "after-swe"):
        blocks[k] = after[k] if after else placeholder
    return blocks


MARK = re.compile(r"(<!-- BEGIN:(?P<name>[A-Za-z0-9_-]+) -->)\n(?P<body>.*?)\n?(<!-- END:(?P=name) -->)", re.S)


def apply(text, blocks):
    used = set()

    def sub(m):
        name = m.group("name")
        if name not in blocks:
            raise SystemExit(f"render.py: report has a block '{name}' that render.py does not produce")
        used.add(name)
        body = blocks[name].strip("\n")
        return f"{m.group(1)}\n{body}\n{m.group(4)}"

    new = MARK.sub(sub, text)
    return new, used


def verify():
    ok = True
    for rs in (RunSet(m) for m in runs_manifest()["runs"]):
        for key, suite in (("suites_abd", rs.abd()), ("suite_c", rs.suite_c())):
            if not suite:
                continue
            for aid, arm in suite["arms"].items():
                s = arm_stats(arm)
                t = arm["totals"]
                for k_stats, k_tot in (("input_tokens", "input_tokens"), ("output_tokens", "output_tokens"), ("tool_calls", "tool_calls"),
                                       ("tool_errors", "tool_errors"), ("model_calls", "model_calls"),
                                       ("agent_wall_total", "agent_wall_secs"), ("wall_total_total", "wall_total_secs")):
                    a, b = s[k_stats], t.get(k_tot)
                    if (a is None) != (b is None) or (a is not None and abs(a - b) > 0.05):
                        print(f"MISMATCH {rs.id} {key} {aid} {k_stats}: rows={a} stored={b}")
                        ok = False
                if s["solved"] != arm["totals"]["solved"]:
                    print(f"MISMATCH {rs.id} {key} {aid} solved")
                    ok = False
    print("verify: totals recomputed from per-task rows match the stored ones" if ok else "verify: FAILED")
    return ok


def main(argv):
    if "--verify" in argv:
        return 0 if verify() else 1
    blocks = build_blocks()
    if "--stdout" in argv:
        for k, v in blocks.items():
            print(f"<!-- {k} -->\n{v}\n")
        return 0
    text = REPORT.read_text(encoding="utf-8")
    new, used = apply(text, blocks)
    missing = sorted(set(blocks) - used)
    if missing:
        print("render.py: blocks produced but not present in the report: " + ", ".join(missing), file=sys.stderr)
    if "--check" in argv:
        if new != text:
            print("render.py: report is out of date; run python3 render.py", file=sys.stderr)
            return 1
        print("render.py: report is up to date")
        return 0
    if new != text:
        REPORT.write_text(new, encoding="utf-8")
        print(f"render.py: rewrote {REPORT.name} ({len(used)} blocks)")
    else:
        print(f"render.py: {REPORT.name} already up to date ({len(used)} blocks)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
