#!/usr/bin/env python3
"""Build a portable, acceptance-gated summary for a Forge long-session run."""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import re
import sqlite3
import sys
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).parent))

import native_long_session as shared


def load_json_lines(path: Path) -> list[dict[str, Any]]:
    records = [
        json.loads(line)
        for line in path.read_text(encoding="utf-8").splitlines()
        if line.strip()
    ]
    if not records:
        raise RuntimeError(f"{path} contains no JSON records")
    return records


def merge_harness_records(records: list[dict[str, Any]]) -> dict[str, Any]:
    session_ids = {
        str(record["session_id"]) for record in records if record.get("session_id")
    }
    if len(session_ids) != 1:
        raise RuntimeError(
            f"harness records must identify one session, got {sorted(session_ids)}"
        )
    turns: dict[int, dict[str, Any]] = {}
    for record in records:
        for turn in record.get("turns") or []:
            number = int(turn["turn"])
            previous = turns.get(number)
            if previous is None or (turn.get("completed") and not previous.get("completed")):
                turns[number] = turn
    attempts = [
        {
            "elapsed_s": record.get("elapsed_s"),
            "timed_out": bool(record.get("timed_out")),
            "timeout_kind": record.get("timeout_kind"),
            "operator_interrupted": bool(record.get("operator_interrupted")),
            "prompt_dispatch_failed": bool(record.get("prompt_dispatch_failed")),
            "turns": [
                {
                    "turn": turn.get("turn"),
                    "completed": bool(turn.get("completed")),
                    "elapsed_s": turn.get("elapsed_s"),
                }
                for turn in record.get("turns") or []
            ],
        }
        for record in records
    ]
    expected = max(int(record.get("turns_expected") or 0) for record in records)
    return {
        "session_id": session_ids.pop(),
        "turns_expected": expected,
        "turns_completed": sum(
            bool(turn.get("completed")) for turn in turns.values()
        ),
        "turns": [turns[number] for number in sorted(turns)],
        "timed_out": any(bool(record.get("timed_out")) for record in records),
        "attempt_elapsed_s": round(
            sum(float(record.get("elapsed_s") or 0.0) for record in records), 3
        ),
        "attempts": attempts,
    }


def prompt_spans(
    messages: list[dict[str, Any]], prompts: list[str]
) -> list[tuple[int, int | None]]:
    user_messages = [message for message in messages if message["role"] == "user"]
    matched: list[int] = []
    previous = -1
    for prompt in prompts:
        digest = hashlib.sha256(prompt.encode()).hexdigest()
        sequence = next(
            (
                int(message["seq"])
                for message in user_messages
                if int(message["seq"]) > previous
                and hashlib.sha256(message["content"].encode()).hexdigest() == digest
            ),
            None,
        )
        if sequence is None:
            raise RuntimeError("session does not contain all six prompts in order")
        matched.append(sequence)
        previous = sequence
    return [
        (sequence, matched[index + 1] if index + 1 < len(matched) else None)
        for index, sequence in enumerate(matched)
    ]


def in_span(sequence: int, start: int, end: int | None) -> bool:
    return sequence > start and (end is None or sequence < end)


def token_totals(rows: list[dict[str, Any]]) -> dict[str, int | float]:
    input_tokens = sum(int(row["input_tokens"]) for row in rows)
    cached = sum(int(row["cached_input_tokens"]) for row in rows)
    output = sum(int(row["output_tokens"]) for row in rows)
    uncached = input_tokens - cached
    return {
        "input_tokens": input_tokens,
        "cached_input_tokens": cached,
        "uncached_input_tokens": uncached,
        "output_tokens": output,
        "raw_tokens": input_tokens + output,
        "cache_adjusted_tokens_025": round(uncached + cached * 0.25 + output, 2),
        "cache_zero_credit_tokens": uncached + output,
    }


def visible_result(path: Path) -> dict[str, Any]:
    text = path.read_text(encoding="utf-8", errors="replace")
    tests = re.search(r"Ran\s+(\d+)\s+tests?", text)
    skipped = re.search(r"skipped=(\d+)", text)
    return {
        "passed": bool(re.search(r"^OK(?:\s|$)", text, re.MULTILINE)),
        "tests_run": int(tests.group(1)) if tests else None,
        "tests_skipped": int(skipped.group(1)) if skipped else 0,
        "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
    }


def quota_result(
    *,
    baseline: float,
    cap: float,
    before: float,
    after: float,
    before_observed_at: str,
    after_observed_at: str,
) -> dict[str, Any]:
    before_time = shared.parse_observed_at(
        before_observed_at, "before_observed_at"
    )
    after_time = shared.parse_observed_at(after_observed_at, "after_observed_at")
    hard_stop = baseline + cap
    errors: list[str] = []
    if not 0.0 <= baseline <= 100.0:
        errors.append("baseline is outside 0..100")
    if cap <= 0.0:
        errors.append("cap delta is not positive")
    if not 0.0 <= before <= 100.0 or not 0.0 <= after <= 100.0:
        errors.append("quota observation is outside 0..100")
    if before < baseline or after < baseline:
        errors.append("quota observation predates or contradicts the baseline")
    if after < before:
        errors.append("post-arm utilization decreased within one quota window")
    if after_time < before_time:
        errors.append("post-arm observation predates the pre-arm observation")
    if after > hard_stop:
        errors.append("post-arm utilization exceeded the hard stop")
    return {
        "baseline_percent": baseline,
        "cap_delta_points": cap,
        "hard_stop_percent": hard_stop,
        "before_percent": before,
        "after_percent": after,
        "arm_delta_points": after - before,
        "cumulative_delta_points": after - baseline,
        "before_observed_at": before_observed_at,
        "after_observed_at": after_observed_at,
        "within_cap": after <= hard_stop,
        "valid": not errors,
        "errors": errors,
    }


def hidden_result_passed(hidden: dict[str, Any]) -> bool:
    return (
        hidden.get("contention_requests") == 100
        and hidden.get("contention_winners") == 1
        and hidden.get("duplicate_requests") == 100
        and hidden.get("concurrent_cancellations") == 100
        and hidden.get("rollback_verified") is True
    )


def original_test_evidence(
    base_tests: dict[str, str],
    final_tests: dict[str, str],
    replay: dict[str, Any],
) -> dict[str, Any]:
    methods_present = all(
        name in final_tests for name in base_tests if name.startswith("test:")
    )
    support_preserved = all(
        final_tests.get(name) == value
        for name, value in base_tests.items()
        if not name.startswith("test:")
    )
    return {
        "original_tests_preserved": all(
            final_tests.get(name) == value for name, value in base_tests.items()
        ),
        "original_test_methods_present": methods_present,
        "original_test_support_preserved": support_preserved,
        "original_tests_not_weakened": (
            replay["exit_code"] == 0
            and not replay["timed_out"]
            and replay["tests_skipped"] == 0
            and methods_present
            and support_preserved
        ),
        "original_test_count": sum(
            name.startswith("test:") for name in base_tests
        ),
        "final_test_count": sum(
            name.startswith("test:") for name in final_tests
        ),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--run-dir", type=Path, required=True)
    parser.add_argument("--db", type=Path, required=True)
    parser.add_argument("--codex-baseline", type=float, required=True)
    parser.add_argument("--codex-cap", type=float, required=True)
    parser.add_argument("--codex-before", type=float, required=True)
    parser.add_argument("--codex-after", type=float, required=True)
    parser.add_argument("--codex-before-observed-at", required=True)
    parser.add_argument("--codex-after-observed-at", required=True)
    parser.add_argument("--claude-baseline", type=float, required=True)
    parser.add_argument("--claude-cap", type=float, required=True)
    parser.add_argument("--claude-before", type=float, required=True)
    parser.add_argument("--claude-after", type=float, required=True)
    parser.add_argument("--claude-before-observed-at", required=True)
    parser.add_argument("--claude-after-observed-at", required=True)
    args = parser.parse_args()

    run_dir = args.run_dir.resolve()
    manifest = shared.load_json(run_dir / "run-manifest.json")
    harness = merge_harness_records(
        load_json_lines(run_dir / "harness-summary.jsonl")
    )
    session_id = harness.get("session_id")
    if not session_id:
        raise RuntimeError("harness summary has no session ID")

    uri = f"file:{args.db.resolve()}?mode=ro"
    with sqlite3.connect(uri, uri=True) as database:
        database.row_factory = sqlite3.Row
        messages = [
            dict(row)
            for row in database.execute(
                """
                SELECT seq, role, visibility, model, content
                FROM message
                WHERE session_id = ?
                ORDER BY seq
                """,
                (session_id,),
            )
        ]
        usage = [
            dict(row)
            for row in database.execute(
                """
                SELECT m.seq, m.role, m.visibility, m.model AS message_model,
                       u.provider, u.model AS usage_model, u.input_tokens,
                       u.cached_input_tokens, u.output_tokens
                FROM usage AS u
                JOIN message AS m ON m.id = u.message_id
                WHERE m.session_id = ?
                ORDER BY m.seq, u.id
                """,
                (session_id,),
            )
        ]
    if not messages:
        raise RuntimeError("Forge session has no persisted messages")

    prompts = shared.load_json(
        shared.SUITE / "scenarios" / manifest["scenario"] / "prompts.json"
    )
    spans = prompt_spans(messages, prompts)
    harness_turns = {
        int(turn["turn"]): turn for turn in harness.get("turns") or []
    }
    turns: list[dict[str, Any]] = []
    for index, (start, end) in enumerate(spans):
        span_messages = [
            message for message in messages if in_span(int(message["seq"]), start, end)
        ]
        span_usage = [row for row in usage if in_span(int(row["seq"]), start, end)]
        routed_models = list(
            dict.fromkeys(
                str(message["model"])
                for message in span_messages
                if message["role"] == "assistant" and message["model"]
            )
        )
        timing = harness_turns.get(index + 1, {})
        turns.append(
            {
                "turn": index + 1,
                "user_seq": start,
                "wall_seconds": timing.get("elapsed_s"),
                "completed": bool(timing.get("completed")),
                "routed_model": routed_models[0] if routed_models else None,
                "models": routed_models,
                "usage_rows": len(span_usage),
                "tokens": token_totals(span_usage),
            }
        )

    totals = token_totals(usage)
    tool_integrity = shared.load_json(run_dir / "session-tool-integrity.json")
    visible = visible_result(run_dir / "visible-tests.log")
    hidden_text = (run_dir / "hidden-tests.log").read_text(
        encoding="utf-8", errors="replace"
    )
    hidden = json.loads(hidden_text.splitlines()[-1])
    workspace = Path(manifest["workspace"])
    scenario_dir = shared.SUITE / "scenarios" / manifest["scenario"]
    marker = scenario_dir / "fixture.source"
    fixture = (
        (scenario_dir / marker.read_text(encoding="utf-8").strip()).resolve()
        if marker.exists()
        else scenario_dir / "fixture"
    )
    signatures_unchanged = shared.public_signatures(
        fixture
    ) == shared.public_signatures(workspace)
    base_tests = shared.preserved_test_contract(fixture)
    final_tests = shared.preserved_test_contract(workspace)
    original_visible = shared.run_verifier(
        [
            sys.executable,
            "-m",
            "unittest",
            "discover",
            "-s",
            str(fixture / "tests"),
            "-v",
        ],
        cwd=workspace,
        path=run_dir / "original-tests-replay.log",
    )
    test_evidence = original_test_evidence(base_tests, final_tests, original_visible)
    base = str(manifest["synthetic_base_commit"])
    patch = shared.capture_patch(workspace, run_dir, base)
    remotes = shared.tiny_command(("git", "remote"), cwd=workspace).splitlines()
    base_is_ancestor = (
        shared.git(
            workspace,
            "merge-base",
            "--is-ancestor",
            base,
            "HEAD",
            check=False,
        ).returncode
        == 0
    )
    quota = {
        "codex": quota_result(
            baseline=args.codex_baseline,
            cap=args.codex_cap,
            before=args.codex_before,
            after=args.codex_after,
            before_observed_at=args.codex_before_observed_at,
            after_observed_at=args.codex_after_observed_at,
        ),
        "claude": quota_result(
            baseline=args.claude_baseline,
            cap=args.claude_cap,
            before=args.claude_before,
            after=args.claude_after,
            before_observed_at=args.claude_before_observed_at,
            after_observed_at=args.claude_after_observed_at,
        ),
    }
    all_passed = (
        harness.get("turns_completed") == 6
        and harness.get("turns_expected") == 6
        and not harness.get("timed_out")
        and len(turns) == 6
        and all(turn["completed"] and turn["routed_model"] for turn in turns)
        and visible["passed"]
        and visible["tests_skipped"] == 0
        and hidden_result_passed(hidden)
        and tool_integrity.get("valid") is True
        and signatures_unchanged
        and test_evidence["original_tests_not_weakened"]
        and patch["diff_check"]
        and not remotes
        and base_is_ancestor
        and manifest.get("model_override") is None
        and manifest.get("effort_override") is None
        and all(provider["valid"] for provider in quota.values())
    )
    result = {
        "schema_version": 1,
        "mode": "forge-full-mesh-auto",
        "forge_version": manifest["forge_version"],
        "model_override": manifest.get("model_override"),
        "effort_override": manifest.get("effort_override"),
        "session_id": session_id,
        "same_session_all_turns": True,
        "harness_attempts": harness["attempts"],
        "total_harness_attempt_wall_seconds": harness["attempt_elapsed_s"],
        "turns": turns,
        "total_turn_wall_seconds": round(
            sum(float(turn["wall_seconds"]) for turn in turns), 3
        ),
        "tokens": totals,
        "visible_tests": visible,
        "hidden_tests": hidden,
        "tool_integrity": tool_integrity,
        "public_signatures_unchanged": signatures_unchanged,
        "original_tests_replay": original_visible,
        **test_evidence,
        "patch": patch,
        "workspace_remotes": remotes,
        "synthetic_base_is_ancestor": base_is_ancestor,
        "synthetic_base_tree": manifest.get("synthetic_base_tree")
        or shared.tiny_command(
            ("git", "rev-parse", f"{base}^{{tree}}"), cwd=workspace
        ),
        "quota": quota,
        "all_acceptance_passed": all_passed,
        "finalized_at": dt.datetime.now(dt.timezone.utc).isoformat(),
    }
    shared.dump_json(run_dir / "final-summary.json", result)
    shared.dump_json(
        run_dir / "session-usage.json",
        {
            "session_id": session_id,
            "turns": turns,
            "totals": totals,
        },
    )
    print(json.dumps(result, sort_keys=True))
    return 0 if all_passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
