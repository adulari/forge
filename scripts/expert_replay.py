#!/usr/bin/env python3
"""Offline replay of the expert-on-call triggers on the advisor gate-0 transcripts.

No model is called. For each baseline Terminal-Bench trial it
  * walks the stored transcript and approximates where the progress supervisor would have
    nudged or stopped (failure, futile and cycle lanes, same thresholds as progress_ledger.rs),
    which is when the stall trigger would have asked the expert;
  * notes whether the DONE trigger would have fired (the turn changed something);
  * answers the DONE consult with the recorded Sonnet review of that trial (advisor-gate0-sonnet),
    standing in for a scripted expert.

The stall replay is an approximation of the Rust ledger (it sees tool results as text, not as
CallObservations), so it states where the real one would be asked, within a few steps.

  python3 scripts/expert_replay.py [--json | --all]
Environment: GATE0_RESULTS (baseline jobs dir), GATE0_SONNET (recorded reviews dir).
"""
import hashlib, json, os, re, sqlite3, sys
from collections import Counter

HOME = os.path.expanduser("~/.cache/forge-ops/harness-frontier")
RESULTS = os.environ.get("GATE0_RESULTS", f"{HOME}/results/baseline-forge/jobs/forge")
SONNET = os.environ.get("GATE0_SONNET", f"{HOME}/advisor-gate0-sonnet")
FAILING = [
    "bun-sourcemap-leak__a6vAFF5", "bun-sourcemap-leak__jorsfpX",
    "cargo-flight-dispatch__Sz7jQ9h", "cargo-flight-dispatch__m9ygT8h",
    "foodstuff-beta-activity__TkeKWvT", "foodstuff-beta-activity__neTWCxk",
    "intrastat-meldung__N87j9AW", "intrastat-meldung__mztkPMf",
    "kv-live-surgery__zfQy9xY", "mvcc-lsm-compaction__ANJFvzC", "roy-polymorph-cn__fFP6wBe",
]
PASSING = ["mvcc-lsm-compaction__UebHWvv", "roy-polymorph-cn__baHpkz7"]

FAILURE_NUDGE, FAILURE_STOP = 3, 6
FUTILE_NUDGE, FUTILE_STOP = 5, 10
CYCLE_NUDGE = 3
MUTATING = {"write_file", "edit_file", "multi_edit", "apply_patch", "append_file", "delete_file"}
WRITES_IN_SHELL = re.compile(r"(>>?\s*\S|\btee\b|sed -i|\.write\(|open\([^)]*['\"][wa]|\bmv\b|\bcp\b|\bmkdir\b)")
MAX_EXPERT_CONSULTS = 2


def steps_of(trial):
    db = sqlite3.connect(f"file:{RESULTS}/{trial}/agent/forge.db?mode=ro", uri=True)
    db.row_factory = sqlite3.Row
    msgs = [dict(r) for r in db.execute(
        "select seq, role, content, tool_calls_json, tool_call_id, nudge from message "
        "where active=1 and visibility in ('llm','llm_only') order by seq")]
    results = {m["tool_call_id"]: m["content"] or "" for m in msgs if m["role"] == "tool"}
    steps = []
    for m in msgs:
        if m["role"] != "assistant" or not m["tool_calls_json"]:
            continue
        calls = json.loads(m["tool_calls_json"])
        steps.append({"seq": m["seq"], "calls": [(c.get("name"), json.dumps(c.get("args"), sort_keys=True),
                                                  results.get(c.get("id"), "")) for c in calls]})
    return steps, msgs


def failed(result):
    m = re.search(r"exit (\d+)", result[:60])
    return bool(m and m.group(1) != "0") or result.lstrip().lower().startswith("error")


def mutates(name, args):
    return name in MUTATING or (name in ("shell", "run_code") and bool(WRITES_IN_SHELL.search(args)))


def supervise(steps):
    """Yield (step_no, 'nudge'|'stop', lane) in the order the ledger would raise them."""
    events, failure_run, futile_run, seen_lines = [], 0, 0, set()
    sig_counts, nudged = Counter(), False
    for n, step in enumerate(steps, 1):
        calls = step["calls"]
        all_failed = bool(calls) and all(failed(r) for _, _, r in calls)
        wrote = any(mutates(name, args) for name, args, _ in calls)
        new_lines = sum(1 for _, _, r in calls for line in r.splitlines() if line.strip() and line not in seen_lines)
        total = sum(1 for _, _, r in calls for line in r.splitlines() if line.strip())
        for _, _, r in calls:
            seen_lines.update(line for line in r.splitlines() if line.strip())
        informative = total and new_lines * 4 >= total
        failure_run = failure_run + 1 if all_failed else 0
        futile_run = 0 if (wrote or informative) else futile_run + 1
        for name, args, r in calls:
            if not wrote:
                sig_counts[(name, args, hashlib.md5(r.encode()).hexdigest())] += 1
        cycle = max(sig_counts.values(), default=0)
        lane = None
        if failure_run >= FAILURE_NUDGE or futile_run >= FUTILE_NUDGE or cycle >= CYCLE_NUDGE:
            lane = "failure" if failure_run >= FAILURE_NUDGE else "futile" if futile_run >= FUTILE_NUDGE else "cycle"
            if not nudged:
                events.append((n, "nudge", lane))
                nudged = True
        if nudged and (failure_run >= FAILURE_STOP or futile_run >= FUTILE_STOP):
            events.append((n, "stop", "failure" if failure_run >= FAILURE_STOP else "futile"))
            break
        if wrote:
            sig_counts.clear()
    return events


def recorded_review(trial):
    verdicts, top = [], None
    for run in range(3):
        path = f"{SONNET}/{trial}.{run}.json"
        if not os.path.exists(path):
            continue
        parsed = json.load(open(path)).get("parsed")
        if not parsed:
            continue
        verdicts.append(parsed["verdict"])
        notes = parsed.get("notes") or []
        if notes and (top is None or (notes[0]["severity"] == "blocker" and top[0] != "blocker")):
            top = (notes[0]["severity"], notes[0]["text"])
    if not verdicts:
        return "no recorded review", None, False
    verdict = Counter(verdicts).most_common(1)[0][0]
    return f"{verdict} ({'/'.join(verdicts)})", top, verdict != "ok"


def replay(trial, kind):
    steps, msgs = steps_of(trial)
    events = supervise(steps)
    changed = any(mutates(name, args) for s in steps for name, args, _ in s["calls"])
    asks = []
    for n, what, lane in events[:MAX_EXPERT_CONSULTS]:
        asks.append({"trigger": "stall", "step": n, "of": len(steps), "before": what, "lane": lane})
    verdict, top, redrive = recorded_review(trial)
    if changed and len(asks) < MAX_EXPERT_CONSULTS:
        asks.append({"trigger": "done", "step": len(steps), "of": len(steps), "before": "finish",
                     "lane": "-", "review": verdict, "top_note": top[1] if top else None,
                     "redrive": redrive})
    return {"trial": trial, "kind": kind, "steps": len(steps), "changed_files": changed, "asks": asks}


def scan_all():
    """Where the stall trigger would fire across every baseline trial, not only the gate-0 set."""
    fired = []
    names = sorted(d for d in os.listdir(RESULTS) if os.path.exists(f"{RESULTS}/{d}/agent/forge.db"))
    for trial in names:
        try:
            steps, _ = steps_of(trial)
        except Exception:
            continue
        events = supervise(steps)
        if events:
            fired.append((trial, len(steps), events[:2]))
    print(f"{len(names)} baseline trials scanned; the stall trigger would have fired on {len(fired)}")
    for trial, n, events in fired:
        print(f"  {trial}: {n} steps; " + ", ".join(f"step {s} {w} ({lane})" for s, w, lane in events))


def main():
    if "--all" in sys.argv:
        return scan_all()
    rows = [replay(t, "failing") for t in FAILING] + [replay(t, "passing") for t in PASSING]
    if "--json" in sys.argv:
        print(json.dumps(rows, indent=1))
        return
    print("| trial | kind | steps | stall asks (step, before, lane) | DONE ask | recorded Sonnet review | re-drive |")
    print("| --- | --- | --- | --- | --- | --- | --- |")
    for r in rows:
        stall = "; ".join(f"{a['step']} {a['before']} {a['lane']}" for a in r["asks"] if a["trigger"] == "stall") or "none"
        done = next((a for a in r["asks"] if a["trigger"] == "done"), None)
        print(f"| {r['trial']} | {r['kind']} | {r['steps']} | {stall} | "
              f"{'step ' + str(done['step']) if done else 'no (nothing changed or cap)'} | "
              f"{done['review'] if done else '-'} | {'yes' if done and done['redrive'] else 'no'} |")
    stalls = sum(1 for r in rows if any(a["trigger"] == "stall" for a in r["asks"]))
    dones = sum(1 for r in rows if any(a["trigger"] == "done" for a in r["asks"]))
    consults = sum(len(r["asks"]) for r in rows)
    print(f"\n{len(rows)} trials: stall trigger fires on {stalls}, DONE trigger on {dones}; "
          f"{consults} expert consults in total (cap {MAX_EXPERT_CONSULTS} per turn).")


if __name__ == "__main__":
    main()
