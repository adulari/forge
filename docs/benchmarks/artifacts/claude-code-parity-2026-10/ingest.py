#!/usr/bin/env python3
"""Turn raw bench-kit output into the JSON files render.py reads. Standard library only.

Used once for the baseline and meant to be reused for every "after fixes" run set:

  ingest.py abd   <aggregate summary.json> <out.json> [--default-run RUN]
  ingest.py c     <aggregate summary.json> <out.json> --runs-dir <results/runs/c> [--failed ARM:TASK:REASON ...]
  ingest.py tasks <suite root> <multiturn sessions dir> <swe instances.jsonl> <swe run-meta.json> <out.json>
  ingest.py env   <out.json> --forge BIN --forge-commit SHA --raw-results <results dir> --archive FILE --archive-sha256 SHA

The aggregate summary is the summary.json that runner/aggregate.py of the bench kit writes. Nothing here
reads or prints credentials. The per-task commitment is the SHA-256 of a deterministic tar of the task
directory, so the hidden tests never enter the repository:

  cd <suite root>/<a|b|c|d> && tar --sort=name --mtime=@0 --owner=0 --group=0 --numeric-owner -cf - <task dir> | sha256sum
"""
import argparse
import datetime
import hashlib
import json
import os
import re
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import render  # noqa: E402  (shared statistics)

TAR = ["tar", "--sort=name", "--mtime=@0", "--owner=0", "--group=0", "--numeric-owner", "-cf", "-"]


def rnd(x, d=3):
    return None if x is None else round(x, d)


def dump(path, obj):
    with open(path, "w", encoding="utf-8") as f:
        json.dump(obj, f, indent=1, ensure_ascii=False)
        f.write("\n")


def task_row(a, note=None):
    r = {
        "solved": bool(a["solved"]),
        "outcome": a["outcome"],
        "timed_out": bool(a["timed_out"]),
        "metrics_complete": bool(a["metrics_complete"]),
        "agent_wall_s": rnd(a["agent_wall"]),
        "wall_total_s": rnd(a["wall_total"]),
        "input_tokens": a["input_tokens"],
        "output_tokens": a["output_tokens"],
        "cached_input_tokens": a["cached_input_tokens"],
        "cache_write_tokens": a["cache_write_tokens"],
        "model_calls": a["model_calls"],
        "tool_calls": a["tool_calls"],
        "tool_errors": a["tool_errors"],
        "ttft_ms": a.get("ttft_ms"),
    }
    if note:
        r["note"] = note
    return r


NOTES = {
    ("CC-deepseek/seed1", "a07-speed-up-log-report"): "timed out at the 1200 s limit; Claude Code reports tokens only when a run ends, so token fields are 0 (tool and model-call counts come from the stream)",
    ("FD-deepseek/seed2", "a05-mailer-options-object"): "no usage row: the Forge session failed to start with 'database is locked'; the concurrent Forge arms shared one store; counted as unsolved; its 47 s agent wall (a failed start) is still part of the speed comparison",
}


def arm_record(aid, src_arm, meta, seed, source_run, rows, totals, medians):
    init = src_arm.get("claude_init") or {}
    return {
        "harness": src_arm["agent"],
        "kind": aid.split("-")[0],
        "model": src_arm["model"],
        "model_resolved": init.get("model") or src_arm["model"],
        "seed": seed,
        "source_run": source_run,
        "clean_isolation": bool(src_arm.get("clean")),
        "neutral_forge_agents_md": bool(meta.get("neutral_forge_agents_md")),
        "timeout_secs": meta.get("timeout_secs"),
        "started_epoch": rnd(meta.get("started"), 1),
        "ended_epoch": rnd(meta.get("ended"), 1),
        "wall_secs": rnd(meta.get("wall_secs"), 1),
        "forge_version": (src_arm.get("versions") or {}).get("forge"),
        "claude_version": (src_arm.get("versions") or {}).get("claude"),
        "claude_init": init or None,
        "invocation": meta.get("invocation"),
        "totals": totals,
        "medians": medians,
        "tasks": rows,
    }


def cmd_abd(args):
    d = json.load(open(args.summary))
    by_arm = {}
    for a in d["attempts"]:
        by_arm.setdefault(a["arm"], []).append(a)
    arms = {}
    for src in d["arms"]:
        name = src["arm"]
        base, _, run = name.partition("@")
        run = run or args.default_run
        m = re.search(r"s(\d+)$", run)
        seed = int(m.group(1)) if m else 1
        aid = f"{base}/seed{seed}"
        rows = {}
        for a in sorted(by_arm[name], key=lambda x: x["task"]):
            rows[a["task"]] = task_row(a, NOTES.get((aid, a["task"])))
        t = src["totals"]
        totals = {k: (rnd(t[k]) if isinstance(t[k], float) else t[k]) for k in
                  ("input_tokens", "output_tokens", "cached_input_tokens", "cache_write_tokens", "tool_calls", "tool_errors",
                   "model_calls", "agent_wall_secs", "wall_total_secs")}
        totals["solved"] = src["solved"]
        totals["attempts"] = src["n_attempts"]
        totals["timeouts"] = src["timeouts"]
        totals["cost_usd_recorded"] = rnd(t["cost_usd"], 6)
        totals["cost_note"] = ("not a price: Claude Code's list-price table applied to a non-Anthropic model" if src.get("cost_untrusted")
                               else "0 for subscription bridge arms; list price for native Claude Code arms; Forge price table for FD")
        medians = {k: rnd(v) for k, v in src["medians"].items()}
        arms[aid] = arm_record(aid, src, d["arm_meta"][name], seed, run, rows, totals, medians)
    for aid, arm in arms.items():
        kind, model = aid.split("/")[0].split("-", 1)
        if kind == "FB":
            partner = next(a for a in arms if a.startswith(f"CC-{model}/"))
            arm["model_resolved"] = arms[partner]["model_resolved"]
            arm["model_resolved_note"] = "read from the model field of the bridged claude process's own transcripts"
    out = {
        "schema": 1,
        "suite": "abd",
        "description": "Suites a, b and d: 26 tasks (9 feature/bugfix/refactor/read, 9 long-horizon/multi-file, 8 harness-robustness).",
        "source": {"file": "results/all-now/summary.json", "generated": d["generated"], "tool": "runner/aggregate.py of the bench kit"},
        "tasks": d["tasks"],
        "notes": d["notes"],
        "arms": arms,
    }
    dump(args.out, out)


def cmd_c(args):
    d = json.load(open(args.summary))
    failed = {}
    for spec in args.failed or []:
        arm, task, reason = spec.split(":", 2)
        failed[(arm, task)] = reason
    by_arm = {}
    for a in d["attempts"]:
        by_arm.setdefault(a["arm"], []).append(a)
    arms = {}
    for src in d["arms"]:
        name = src["arm"]
        base, _, task = name.partition("@")
        aid = f"{base}/seed1"
        if aid not in arms:
            arms[aid] = {"src": src, "meta": d["arm_meta"][name], "rows": {}, "totals": {}, "raw": []}
        rec = arms[aid]
        rec["raw"].append((task, src))
        att = by_arm.get(name, [])
        run_dir = Path(args.runs_dir) / task / base / "seed1"
        score = None
        for lf in sorted((run_dir / "tasks.logs").glob("*.test.log")) if (run_dir / "tasks.logs").exists() else []:
            m = re.search(r"^SCORE (\d+/\d+)", lf.read_text(errors="replace"), re.M)
            if m:
                score = m.group(1)
        if att:
            row = task_row(att[0])
            if score:
                row["grader_score"] = score
        else:
            seed = json.load(open(run_dir / "seed.json"))
            row = {"solved": False, "outcome": "no_result", "reason": failed.get((base, task), "no result row"), "timed_out": False,
                   "metrics_complete": False, "agent_wall_s": None, "wall_total_s": None, "input_tokens": None, "output_tokens": None,
                   "cached_input_tokens": None, "cache_write_tokens": None, "model_calls": None, "tool_calls": None, "tool_errors": None,
                   "ttft_ms": None, "process_exit_code": seed["exit_code"], "process_wall_s": rnd(seed["wall_secs"], 1)}
        rec["rows"][task] = row
    out_arms = {}
    for aid, rec in arms.items():
        rows = dict(sorted(rec["rows"].items()))
        # totals: independent path, the sum of the per-task aggregate summaries
        keys = ("input_tokens", "output_tokens", "cached_input_tokens", "cache_write_tokens", "tool_calls", "tool_errors",
                "model_calls", "agent_wall_secs", "wall_total_secs")
        totals = {}
        for k in keys:
            v = [s["totals"][k] for _, s in rec["raw"] if s["totals"].get(k) is not None]
            totals[k] = rnd(sum(v)) if v else None
        totals["solved"] = sum(s["solved"] for _, s in rec["raw"])
        totals["attempts"] = len(rows)
        totals["timeouts"] = sum(s["timeouts"] for _, s in rec["raw"])
        stats = render.arm_stats({"tasks": rows})
        medians = {"agent_wall": rnd(stats["agent_wall_median"]), "wall_total": rnd(stats["wall_total_median"]),
                   "input_tokens": stats["input_median"], "output_tokens": stats["output_median"], "tool_calls": stats["tool_calls_median"]}
        arm_obj = arm_record(aid, rec["src"], rec["meta"], 1, "c", rows, totals, medians)
        arm_obj["note"] = ("Each task ran as its own arm directory (one process, its own store); rows are merged per harness. "
                           "FD c06 has no result row and is counted as failed.") if aid.startswith("FD") else (
            "Each task ran as its own arm directory (one process per task); rows are merged per harness.")
        out_arms[aid] = arm_obj
    out = {
        "schema": 1,
        "suite": "c",
        "description": "Suite c: 11 large-codebase tasks (Rust ~390k LOC, Python, JS), one attempt per task and harness.",
        "source": {"file": "results/c-agg/summary.json", "generated": d["generated"], "tool": "runner/aggregate.py of the bench kit"},
        "tasks": d["tasks"],
        "notes": d["notes"],
        "arms": out_arms,
    }
    dump(args.out, out)


# ----------------------------------------------------------------------------- tasks
SUMMARIES = {
    "a01-fix-back-to-back-conflicts": ("Python", "bugfix", "A room scheduler reports back-to-back bookings as conflicts; fix it to match the README without editing existing tests."),
    "a02-sliding-window-limiter": ("Python", "feature", "Implement a sliding-window rate limiter from an approved spec, following the layout of the existing TokenBucket."),
    "a03-refactor-order-pricing": ("JavaScript", "refactor", "Split a 170-line order-total function into discount, tax and shipping modules behind a short orchestrator."),
    "a04-debug-nightly-export-crash": ("Python", "debugging", "Find and fix the root cause of a nightly export crash on a finance CSV; no skipped rows, input untouched."),
    "a05-mailer-options-object": ("TypeScript", "refactor", "Replace sendMail's six positional parameters with one options object and update every call site."),
    "a06-answer-retry-policy": ("JavaScript", "code reading (no edits)", "Read a sync agent and answer questions about its production retry policy in ANSWER.md; change no code."),
    "a07-speed-up-log-report": ("Python", "performance", "Make an access-log report fast on ~100k lines with byte-identical output, standard library only."),
    "a08-signup-validation-constraints": ("JavaScript", "feature", "Add input validation to a signup handler (name, email, password rules; case-insensitive duplicate check)."),
    "a09-layered-config-cli": ("Python", "feature", "Layer a CLI's settings (flags, environment, TOML file, defaults) and add a command to show the resolved values."),
    "b01-stockctl-longhorizon": ("Rust", "multi-step feature", "Five ordered requests to a Rust CLI: --min-qty filter, per-category summary, export command, format error, README and tests."),
    "b02-ttlcache-expiry-trap": ("Go", "bugfix", "Make Cache.Get never return an expired entry, keeping existing tests green."),
    "b03-rename-account-wire-frozen": ("Go", "cross-cutting refactor", "Rename the User concept to Account across a billing service while it still builds and tests pass."),
    "b04-batch-run-concurrency": ("Go", "bugfix (concurrency)", "Make batch.Run preserve order, pass the race detector and stop hanging after a failed call, without changing its signature."),
    "b05-minimd-broken-merge": ("Rust", "merge repair", "Repair a broken merge so the project builds and tests pass offline, with blockquotes behaving as the README says."),
    "b06-semver-regression-archaeology": ("Go", "git archaeology + bugfix", "Find the commit that broke release-candidate ordering, fix it without reverting newer work, add a test and a changelog line."),
    "b07-ledger-rollup-awk": ("Bash/awk", "scripting", "Write a plain bash and POSIX-tools script that rolls bank-export CSVs up into monthly category totals."),
    "b08-scaffold-service-noninteractive": ("Go + shell", "tooling", "Create a new service through the repo's scaffolding script (next free port, owners) and make the registry check pass."),
    "b09-fsweep-docs-drift": ("Rust", "documentation sync", "Bring README, usage docs and --help text in line with what the tool actually does, without changing behaviour."),
    "d01-interactive-test-runner": ("Python + shell", "robustness (interactive prompt)", "Fix a duration parser so the project's own test runner passes; the runner prompts interactively unless told not to."),
    "d02-huge-output-needle": ("shell + Python", "robustness (huge output)", "Find one failing batch id in a tens-of-megabytes trace and write it to answer.txt."),
    "d03-flaky-loading-cache": ("Python", "robustness (flaky test, race)", "A test fails about one run in three; fix the real race in the library, not the test."),
    "d04-long-build-await": ("shell", "robustness (long-running command)", "Cut a release: set version and channel, run the multi-minute build and leave a complete dist/."),
    "d05-background-server-roundtrip": ("Python", "robustness (process lifecycle)", "Start a local service, fetch a token from its API, save it, and shut the service down cleanly."),
    "d06-readonly-config-files": ("shell", "robustness (file permissions)", "Raise a limit in a config file that is mode 0444 and make the checker print OK."),
    "d07-binary-neighbor-schema": ("text + binary data", "robustness (large binary neighbour)", "Update a stale schema document to match a large binary data file's header, touching nothing else."),
    "d08-bsd-sed-make-target": ("Make + YAML", "robustness (portability)", "Set every service to debug logging through a make target that was written for BSD sed and date."),
    "c01-pinned-429-wait-budget": ("Rust (Forge snapshot)", "code reading + computation", "Answer questions about the retry and wait budget for rate-limited pinned requests across three crates (ANSWER.md)."),
    "c02-compaction-trigger-budget": ("Rust (Forge snapshot)", "code reading + computation", "Trace the compaction trigger from its config default to the token budgets and compute them for a given window (ANSWER.md)."),
    "c03-provider-implementors": ("Rust (Forge snapshot)", "exhaustive search + counts", "Enumerate the production implementors of the Provider trait and answer count questions (ANSWER.md)."),
    "c04-claude-bridge-spawn-path": ("Rust (Forge snapshot)", "data-flow tracing", "Trace how a model id becomes the arguments and environment of the spawned claude process (ANSWER.md)."),
    "c05-effort-ceiling-value-rung": ("Rust (Forge snapshot)", "bugfix (silent tests)", "Find and fix an off-by-one in an effort-ceiling selection function that the existing tests do not catch."),
    "c06-rename-config-provider-of": ("Rust (Forge snapshot)", "cross-crate rename", "Rename one config function across crates without touching same-named functions elsewhere; the crate must still compile."),
    "c07-http-conditional-leap-march": ("Python (Twisted)", "bugfix (deep call chain)", "Fix wrong HTTP conditional-request behaviour reported only as a symptom."),
    "c08-gitignore-negation-parent-dir": ("Python (dulwich)", "bugfix (deep call chain)", "Fix a .gitignore negation behaviour reported only as a symptom; the prompt says not to edit existing files under dulwich/tests."),
    "c09-npm-config-hash-value": ("JavaScript (npm)", "bugfix (deep call chain)", "Fix an npm config round trip that breaks on a value containing a hash character."),
    "c10-dulwich-commit-to-loose-object": ("Python (dulwich)", "data-flow tracing", "Trace the path from creating a commit to the loose object file on disk (ANSWER.md)."),
    "c11-tool-output-truncation": ("Rust (Forge snapshot)", "code reading + computation", "Compare two tool-output truncation functions and compute how each counts lines (ANSWER.md)."),
}
MT_SUMMARIES = {
    "S1-tasklog": ("Python", "Seven prompts building a task tracker CLI feature by feature under a standing constraint set in prompt 2."),
    "S2-fxbatch": ("Python", "Seven prompts on a batch currency converter; prompt 5 reverses an earlier design decision."),
    "S3-invoicer": ("Python", "Seven prompts debugging three reported bugs, with later prompts referring back to earlier findings."),
}


def tar_sha(cwd, name):
    p1 = subprocess.run(TAR + [name], cwd=cwd, stdout=subprocess.PIPE, check=True)
    return hashlib.sha256(p1.stdout).hexdigest()


def file_sha(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def cmd_tasks(args):
    root = Path(args.suite_root)
    tasks = []
    for letter in "abcd":
        for d in sorted((root / letter).iterdir()):
            if not d.is_dir() or d.name.startswith("_"):
                continue
            tid = d.name if letter == "c" else f"{letter}{d.name}"
            lang, ttype, summary = SUMMARIES[tid]
            tasks.append({"id": tid, "suite": letter, "dir": f"{letter}/{d.name}", "language": lang, "type": ttype,
                          "summary": summary, "sha256": tar_sha(root / letter, d.name)})
    missing = set(SUMMARIES) - {t["id"] for t in tasks}
    if missing:
        raise SystemExit(f"summaries for tasks that do not exist: {sorted(missing)}")
    shared = []
    for sub in ("a/_src", "a/_tools", "b/_tools", "c/_tools"):
        p = root / sub
        if p.exists():
            shared.append({"path": f"suite/{sub}", "kind": "directory", "sha256": tar_sha(root / Path(sub).parent, Path(sub).name)})
    for f in sorted((root / "c/_src").iterdir()):
        shared.append({"path": f"suite/c/_src/{f.name}", "kind": "file", "sha256": file_sha(f)})
    mt = []
    sess_root = Path(args.sessions)
    for d in sorted(sess_root.iterdir()):
        if d.name not in MT_SUMMARIES:
            continue
        lang, summ = MT_SUMMARIES[d.name]
        mt.append({"id": d.name, "language": lang, "summary": summ, "dir": f"multiturn/sessions/{d.name}", "sha256": tar_sha(sess_root, d.name)})
    inst = []
    for line in open(args.swe_instances):
        r = json.loads(line)
        inst.append({"id": r["instance_id"], "repo": r["repo"], "version": r["version"], "difficulty": r["difficulty"],
                     "language": "Python"})
    meta = json.load(open(args.swe_meta))
    out = {
        "schema": 1,
        "commitment": {
            "what": "SHA-256 of a deterministic tar of the task directory (instruction, setup, hidden grader, reference solution, negative controls). The directories are not published.",
            "command": "cd <suite root>/<suite letter dir> && tar --sort=name --mtime=@0 --owner=0 --group=0 --numeric-owner -cf - <dir basename> | sha256sum",
            "tar": subprocess.run(["tar", "--version"], capture_output=True, text=True).stdout.splitlines()[0],
            "suite_letter_dirs": "a, b, c, d; the `dir` field gives <letter>/<dir basename>. Task ids for a, b and d are the letter plus the directory name.",
        },
        "tasks": tasks,
        "shared_inputs": shared,
        "multiturn": mt,
        "swe": {"dataset": "SWE-bench Verified, 10 instances (public)", "instances_sha256": meta["instances_sha256"], "instances": inst},
    }
    dump(args.out, out)


# ----------------------------------------------------------------------------- environment
def sh(cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, **kw).stdout.strip()


def iso(epoch):
    return datetime.datetime.fromtimestamp(epoch, datetime.timezone.utc).strftime("%Y-%m-%d %H:%MZ")


def cmd_env(args):
    raw = Path(args.raw_results)
    lscpu = {k.strip(): v.strip() for k, _, v in (ln.partition(":") for ln in sh(["lscpu"]).splitlines())}
    mem = int(sh(["bash", "-c", "free -b | awk '/^Mem:/{print $2}'"]))
    osrel = dict(ln.split("=", 1) for ln in open("/etc/os-release").read().splitlines() if "=" in ln)
    venv_py = raw.parent / "swe/.venv/bin/python"
    swe_ver = sh([str(venv_py), "-c", "import swebench; print(swebench.__version__)"])
    abd = json.load(open(raw / "all-now/summary.json"))
    starts = [m["started"] for m in abd["arm_meta"].values()]
    ends = [m["ended"] for m in abd["arm_meta"].values()]
    cseeds = [json.load(open(p)) for p in sorted((raw / "runs/c").glob("c*/*-deepseek/seed1/seed.json"))]
    swe_end = {a: iso(datetime.datetime.strptime(json.load(open(raw / f"swe/{a}-deepseek/run-meta.json"))["finished_at"],
                                                 "%Y-%m-%dT%H:%M:%S%z").timestamp()) for a in ("CC", "FD")}
    mt_end = {a: iso(os.path.getmtime(raw / f"mt/{a}-deepseek/summary.json")) for a in ("CC", "FD")}
    models = sorted({(a.get("claude_init") or {}).get("model") for a in abd["arms"]} - {None})
    models.append("inferhub::cb/deepseek-v4.1-flash (Forge FD arm, provider id)")
    out = {
        "schema": 1,
        "forge": {
            "version": sh([args.forge, "--version"]).replace("forge ", ""),
            "commit": args.forge_commit,
            "commit_provenance": ("built by the kit's forge-dev-update from this commit: its log reads `building 8b18cad8 fix(push): keep generic iOS alerts until replaced; warn on failed APNs sends (#1601)` and was written at 2026-10-21 13:00:43 +0200, within a second of the binary's modification time, and the pinned copy has the same SHA-256 as the binary that build produced. The binary itself embeds no commit hash"),
            "binary": "ccbench/bin/forge (pinned copy of the release build used for every Forge arm)",
            "binary_sha256": file_sha(args.forge),
        },
        "claude_code": {"version": sh(["claude", "--version"])},
        "models_used": models,
        "deepseek_provider": "InferHub (https://api.inferhub.dev): Forge arm through its inferhub provider, Claude Code arm through InferHub's Anthropic-compatible /v1/messages endpoint",
        "swebench": {"version": swe_ver, "note": "official swebench.harness.run_evaluation, prebuilt instance images, 4 workers"},
        "docker": sh(["docker", "--version"]).replace("Docker version ", "").split(",")[0],
        "python": f"{sh(['python3', '--version']).replace('Python ', '')} (host); {sh([str(venv_py), '--version']).replace('Python ', '')} (SWE-bench venv)",
        "host": {
            "cpu": lscpu["Model name"], "cores": int(lscpu["CPU(s)"]), "ram": f"{mem / 2**30:.0f} GiB",
            "os": osrel["PRETTY_NAME"].strip('"'), "kernel": sh(["uname", "-sr"]),
        },
        "dates": {
            "window_utc": "2026-10-22, 12:21Z to 14:27Z",
            "suites_abd": f"{iso(min(starts))} to {iso(max(ends))}",
            "suite_c": f"{iso(min(s['started'] for s in cseeds))} to {iso(max(s['ended'] for s in cseeds))} (CC and FD arms only)",
            "multiturn": f"finished {mt_end['CC']} (CC), {mt_end['FD']} (FD)",
            "swe": f"finished {swe_end['CC']} (CC), {swe_end['FD']} (FD)",
        },
        "concurrency": ("The eight Haiku and DeepSeek arms of suites a+b+d were started within a millisecond of each other (12:21Z) and ran side by side; the Haiku arms ended by 12:56Z and the DeepSeek arms between 13:05Z and 13:15Z. The Sonnet pair started at 13:11Z, while the DeepSeek arms were still finishing, and ended at 13:28Z and 13:29Z. Suite c (one chain per harness), the multi-turn sessions (both harnesses) and SWE-bench (both harnesses, two instances at a time each) all started together at 13:25Z, so they overlapped the last minutes of the Sonnet pair. The Claude Code chain of suite c ended at 13:43Z and the Forge chain at 13:56Z; multi-turn ended at 13:39Z (CC) and 13:40Z (FD), SWE-bench at 13:51Z (CC) and 14:27Z (FD). The machine also hosted other agents' work, and load was not recorded, so absolute wall times carry noise."),
        "private_archive": {"file": args.archive, "sha256": args.archive_sha256},
    }
    dump(args.out, out)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("abd")
    p.add_argument("summary")
    p.add_argument("out")
    p.add_argument("--default-run", default="s1")
    p.set_defaults(fn=cmd_abd)
    p = sub.add_parser("c")
    p.add_argument("summary")
    p.add_argument("out")
    p.add_argument("--runs-dir", required=True)
    p.add_argument("--failed", action="append")
    p.set_defaults(fn=cmd_c)
    p = sub.add_parser("tasks")
    p.add_argument("suite_root")
    p.add_argument("sessions")
    p.add_argument("swe_instances")
    p.add_argument("swe_meta")
    p.add_argument("out")
    p.set_defaults(fn=cmd_tasks)
    p = sub.add_parser("env")
    p.add_argument("out")
    p.add_argument("--forge", required=True)
    p.add_argument("--forge-commit", required=True)
    p.add_argument("--raw-results", required=True)
    p.add_argument("--archive", required=True)
    p.add_argument("--archive-sha256", required=True)
    p.set_defaults(fn=cmd_env)
    args = ap.parse_args()
    args.fn(args)


if __name__ == "__main__":
    main()
