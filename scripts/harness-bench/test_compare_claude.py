#!/usr/bin/env python3

from __future__ import annotations

import contextlib
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import compare_claude_swe as claude


class ClaudeBenchmarkTests(unittest.TestCase):
    def test_cli_rejects_multiple_paid_arms_per_refresh(self) -> None:
        argv = [
            "compare_claude_swe.py",
            "--dataset",
            "dataset.jsonl",
            "--out",
            "out",
            "--worktree-root",
            "worktrees",
            "--baseline-weekly-pct",
            "20",
            "--observed-weekly-pct",
            "20",
            "--max-new-trials",
            "2",
        ]
        with (
            mock.patch.object(sys, "argv", argv),
            contextlib.redirect_stderr(io.StringIO()),
            self.assertRaises(SystemExit) as raised,
        ):
            claude.main()

        self.assertEqual(raised.exception.code, 2)

    def test_single_native_arm_is_a_complete_selected_arm_set(self) -> None:
        trials = claude.swe.plan_trials(
            ("sonnet",),
            ({"instance_id": "owner__repo-1"},),
            ("raw-claude",),
            42,
        )
        self.assertEqual(len(trials), 1)
        self.assertEqual(trials[0].arm, "raw-claude")

    def test_claude_cache_counters_are_additive(self) -> None:
        metrics = claude.claude_token_metrics(
            {
                "input_tokens": 100,
                "cache_read_input_tokens": 800,
                "cache_creation_input_tokens": 50,
                "output_tokens": 25,
            }
        )
        self.assertEqual(metrics["input_tokens"], 950)
        self.assertEqual(metrics["total_tokens"], 975)
        self.assertEqual(metrics["cached_input_tokens"], 800)
        self.assertEqual(metrics["cache_adjusted_tokens_025"], 375)

    def test_raw_summary_reads_usage_tools_and_weekly_quota(self) -> None:
        events = [
            {
                "type": "assistant",
                "message": {
                    "content": [
                        {"type": "tool_use", "id": "t1", "name": "Read", "input": {}}
                    ]
                },
            },
            {
                "type": "user",
                "message": {
                    "content": [
                        {"type": "tool_result", "tool_use_id": "t1", "content": "ok"}
                    ]
                },
            },
            {
                "type": "rate_limit_event",
                "rate_limit_info": {
                    "rateLimitType": "seven_day",
                    "utilization": 0.21,
                    "resetsAt": 123,
                    "status": "allowed",
                },
            },
            {
                "type": "result",
                "subtype": "success",
                "session_id": "s1",
                "num_turns": 3,
                "result": "done",
                "usage": {
                    "input_tokens": 10,
                    "cache_read_input_tokens": 20,
                    "cache_creation_input_tokens": 5,
                    "output_tokens": 4,
                },
            },
        ]
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "events.jsonl"
            path.write_text(
                "\n".join(json.dumps(event) for event in events) + "\n",
                encoding="utf-8",
            )
            summary = claude.summarize_raw_claude_events(path)
        self.assertEqual(summary["session_id"], "s1")
        self.assertEqual(summary["tool_uses"], 1)
        self.assertEqual(summary["tool_results"], 1)
        self.assertEqual(summary["tokens"]["total_tokens"], 39)
        self.assertEqual(summary["quota"]["used_percent"], 21)

    def test_effective_quota_never_ignores_fresher_external_value(self) -> None:
        summaries = [{"quota": {"used_percent": 22.0}}]
        self.assertEqual(claude.effective_quota(summaries, 24.0), 24.0)

    def test_resolves_current_aliases_from_authoritative_capabilities(self) -> None:
        runtime = {
            "models": [
                {
                    "value": "opus[1m]",
                    "resolvedModel": "claude-opus-5[1m]",
                    "displayName": "Opus (1M context)",
                },
                {
                    "value": "sonnet",
                    "resolvedModel": "claude-sonnet-5",
                    "displayName": "Sonnet",
                },
            ]
        }
        self.assertEqual(
            claude.resolve_claude_model(runtime, "opus")["resolvedModel"],
            "claude-opus-5[1m]",
        )
        self.assertEqual(
            claude.resolve_claude_model(runtime, "sonnet")["resolvedModel"],
            "claude-sonnet-5",
        )


if __name__ == "__main__":
    unittest.main()
