import argparse
import contextlib
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).parent))

import native_long_session as native


class NativeLongSessionTests(unittest.TestCase):
    def test_command_builders_pin_regular_high_and_resume_the_same_session(
        self,
    ) -> None:
        workspace = Path("/tmp/native-stress-workspace")
        first = native.codex_argv("gpt-5.6-sol", "high", "prompt", workspace, None)
        resumed = native.codex_argv(
            "gpt-5.6-sol", "high", "prompt", workspace, "codex-session"
        )
        self.assertEqual(first[:2], ["codex", "exec"])
        self.assertIn('model_reasoning_effort="high"', first)
        self.assertEqual(first[first.index("--sandbox") + 1], "workspace-write")
        self.assertIn("standalone_web_search", first)
        self.assertNotIn("--dangerously-bypass-approvals-and-sandbox", first)
        self.assertNotIn("xhigh", first)
        self.assertEqual(resumed[:3], ["codex", "exec", "resume"])
        self.assertIn("codex-session", resumed)
        self.assertNotIn("--sandbox", resumed)
        self.assertIn('sandbox_mode="workspace-write"', resumed)

        claude_first = native.claude_argv(
            "opus", "high", "prompt", "claude-session", True
        )
        claude_resumed = native.claude_argv(
            "opus", "high", "prompt", "claude-session", False
        )
        self.assertIn("--session-id", claude_first)
        self.assertIn("--resume", claude_resumed)
        self.assertIn("--strict-mcp-config", claude_first)
        self.assertEqual(
            claude_first[claude_first.index("--disallowedTools") + 1],
            "WebFetch,WebSearch",
        )
        self.assertTrue(
            any('"strictAllowlist":true' in argument for argument in claude_first)
        )
        self.assertEqual(claude_first[claude_first.index("--effort") + 1], "high")
        self.assertNotIn("xhigh", claude_first)

    def test_token_normalization_uses_full_input_and_two_cache_sensitivities(
        self,
    ) -> None:
        codex = native.codex_tokens(
            {
                "input_tokens": 1000,
                "cached_input_tokens": 800,
                "output_tokens": 100,
            }
        )
        self.assertEqual(codex["total_tokens"], 1100)
        self.assertEqual(codex["cache_adjusted_tokens_025"], 500)
        self.assertEqual(codex["cache_zero_credit_tokens"], 300)

        claude = native.claude_tokens(
            {
                "input_tokens": 100,
                "cache_read_input_tokens": 800,
                "cache_creation_input_tokens": 100,
                "output_tokens": 100,
            }
        )
        self.assertEqual(claude["input_tokens"], 1000)
        self.assertEqual(claude["total_tokens"], 1100)
        self.assertEqual(claude["cache_adjusted_tokens_025"], 500)

    def test_event_summaries_recover_session_model_usage_and_tokens(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            codex_path = root / "codex.jsonl"
            codex_path.write_text(
                "\n".join(
                    json.dumps(event)
                    for event in [
                        {"type": "thread.started", "thread_id": "thread-1"},
                        {
                            "type": "item.completed",
                            "item": {
                                "type": "command_execution",
                                "command": "python -m unittest",
                            },
                        },
                        {
                            "type": "item.completed",
                            "item": {"type": "agent_message", "text": "done"},
                        },
                        {
                            "type": "turn.completed",
                            "usage": {
                                "input_tokens": 20,
                                "cached_input_tokens": 8,
                                "output_tokens": 4,
                            },
                        },
                    ]
                )
                + "\n",
                encoding="utf-8",
            )
            codex = native.summarize_codex(codex_path)
            self.assertTrue(codex["completed"])
            self.assertEqual(codex["session_id"], "thread-1")
            self.assertEqual(codex["tool_calls"], 1)
            self.assertEqual(codex["tokens"]["total_tokens"], 24)

            claude_path = root / "claude.jsonl"
            claude_path.write_text(
                json.dumps(
                    {
                        "type": "result",
                        "session_id": "claude-1",
                        "is_error": False,
                        "result": "done",
                        "modelUsage": {"claude-opus-5-20260701": {}},
                        "usage": {
                            "input_tokens": 5,
                            "cache_read_input_tokens": 10,
                            "cache_creation_input_tokens": 3,
                            "output_tokens": 2,
                        },
                    }
                )
                + "\n",
                encoding="utf-8",
            )
            claude = native.summarize_claude(claude_path)
            self.assertTrue(claude["completed"])
            self.assertEqual(claude["resolved_models"], ["claude-opus-5-20260701"])
            self.assertEqual(claude["tokens"]["total_tokens"], 20)

    def test_execution_evidence_requires_exact_model_and_regular_effort(
        self,
    ) -> None:
        codex_state = {
            "provider": "codex",
            "expected_resolved_model": "gpt-5.6-sol",
            "effort": "high",
        }
        self.assertIsNone(
            native.execution_evidence_error(
                codex_state,
                {
                    "resolved_model": "gpt-5.6-sol",
                    "resolved_effort": "high",
                },
            )
        )
        self.assertIn(
            "did not match",
            native.execution_evidence_error(
                codex_state,
                {
                    "resolved_model": "gpt-5.6-terra",
                    "resolved_effort": "high",
                },
            )
            or "",
        )
        self.assertIn(
            "effort",
            native.execution_evidence_error(
                codex_state,
                {
                    "resolved_model": "gpt-5.6-sol",
                    "resolved_effort": "xhigh",
                },
            )
            or "",
        )
        self.assertIn(
            "required regular",
            native.execution_evidence_error(
                {
                    "provider": "codex",
                    "expected_resolved_model": "gpt-5.6-sol",
                    "effort": "medium",
                },
                {
                    "resolved_model": "gpt-5.6-sol",
                    "resolved_effort": "medium",
                },
            )
            or "",
        )

        claude_state = {
            "provider": "claude",
            "expected_resolved_model": "claude-opus-5[1m]",
            "effort": "high",
        }
        self.assertIsNone(
            native.execution_evidence_error(
                claude_state,
                {"resolved_models": ["claude-opus-5[1m]"]},
            )
        )
        self.assertIn(
            "sole expected model",
            native.execution_evidence_error(
                claude_state,
                {"resolved_models": ["claude-sonnet-5"]},
            )
            or "",
        )
        self.assertIn(
            "not configured",
            native.execution_evidence_error(
                {"provider": "claude", "effort": "high"},
                {"resolved_models": ["claude-opus-5[1m]"]},
            )
            or "",
        )
        with tempfile.TemporaryDirectory() as directory:
            out_root = Path(directory)
            with self.assertRaisesRegex(ValueError, "expected-resolved-model"):
                native.prepare(
                    argparse.Namespace(
                        provider="claude",
                        model="opus[1m]",
                        effort="high",
                        scenario=native.DEFAULT_SCENARIO,
                        out_root=out_root,
                        quota_baseline=27.0,
                        quota_cap=5.0,
                    )
                )
            self.assertEqual(list(out_root.iterdir()), [])
            with self.assertRaisesRegex(ValueError, "regular high"):
                native.prepare(
                    argparse.Namespace(
                        provider="codex",
                        model="gpt-5.6-sol",
                        effort="medium",
                        scenario=native.DEFAULT_SCENARIO,
                        out_root=out_root,
                        quota_baseline=36.0,
                        quota_cap=10.0,
                    )
                )
            self.assertEqual(list(out_root.iterdir()), [])

    def test_quota_gate_requires_fresh_post_turn_observation_and_fails_at_cap(
        self,
    ) -> None:
        state = {
            "turns": [{}],
            "quota_baseline_percent": 36,
            "quota_cap_delta_points": 10,
            "quota_observations": [
                {
                    "recorded_at": native.utc_now(),
                    "external_observed_at": native.utc_now(),
                    "weekly_utilization_percent": 37,
                    "after_turn": 0,
                }
            ],
        }
        valid, reason = native.quota_is_fresh(state, 900)
        self.assertFalse(valid)
        self.assertIn("post-turn", reason)

        state["quota_observations"][-1]["after_turn"] = 1
        state["turns"][0]["process"] = {
            "ended_at": (
                native.parse_observed_at(native.utc_now(), "now")
                - native.dt.timedelta(seconds=1)
            ).isoformat()
        }
        self.assertTrue(native.quota_is_fresh(state, 900)[0])
        state["turns"][0]["process"]["ended_at"] = (
            native.parse_observed_at(native.utc_now(), "now")
            + native.dt.timedelta(seconds=10)
        ).isoformat()
        valid, reason = native.quota_is_fresh(state, 900)
        self.assertFalse(valid)
        self.assertIn("predates", reason)
        state["turns"][0]["process"]["ended_at"] = (
            native.parse_observed_at(native.utc_now(), "now")
            - native.dt.timedelta(seconds=1)
        ).isoformat()
        state["quota_observations"][-1]["weekly_utilization_percent"] = 46
        valid, reason = native.quota_is_fresh(state, 900)
        self.assertFalse(valid)
        self.assertIn("hard stop", reason)

        self.assertTrue(
            native.quota_is_fresh(state, 900, for_paid_call=False)[0],
            "a final result exactly at the allowed cap must remain finalizable",
        )
        state["quota_observations"][-1]["weekly_utilization_percent"] = 46.1
        self.assertFalse(
            native.quota_is_fresh(state, 900, for_paid_call=False)[0],
            "an actual cap overshoot must invalidate finalization",
        )

    def test_quota_gate_rejects_decreasing_and_out_of_order_ledgers(self) -> None:
        now = native.parse_observed_at(native.utc_now(), "now")
        state = {
            "turns": [],
            "quota_baseline_percent": 35,
            "quota_cap_delta_points": 5,
            "quota_observations": [
                {
                    "recorded_at": (now - native.dt.timedelta(seconds=2)).isoformat(),
                    "external_observed_at": (
                        now - native.dt.timedelta(seconds=3)
                    ).isoformat(),
                    "weekly_utilization_percent": 36,
                    "after_turn": 0,
                },
                {
                    "recorded_at": now.isoformat(),
                    "external_observed_at": (
                        now - native.dt.timedelta(seconds=1)
                    ).isoformat(),
                    "weekly_utilization_percent": 37,
                    "after_turn": 0,
                },
            ],
        }
        self.assertTrue(native.quota_is_fresh(state, 900)[0])

        state["quota_observations"][-1]["weekly_utilization_percent"] = 35.5
        valid, reason = native.quota_is_fresh(state, 900)
        self.assertFalse(valid)
        self.assertIn("decreased", reason)

        state["quota_observations"][-1]["weekly_utilization_percent"] = 37
        state["quota_observations"][-1]["external_observed_at"] = (
            now - native.dt.timedelta(seconds=4)
        ).isoformat()
        valid, reason = native.quota_is_fresh(state, 900)
        self.assertFalse(valid)
        self.assertIn("out of order", reason)

    def test_integrity_audit_hashes_evidence_without_echoing_private_commands(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as directory:
            run_dir = Path(directory)
            turn = run_dir / "turns" / "01"
            turn.mkdir(parents=True)
            command = "curl https://example.test/solution"
            (turn / "events.jsonl").write_text(
                json.dumps(
                    {
                        "type": "item.completed",
                        "item": {"type": "command_execution", "command": command},
                    }
                )
                + "\n",
                encoding="utf-8",
            )

            audit = native.integrity_audit(run_dir)

            self.assertFalse(audit["valid"])
            serialized = json.dumps(audit)
            self.assertNotIn(command, serialized)
            self.assertIn("text_sha256", serialized)

    def test_integrity_audit_ignores_prose_and_output_but_checks_claude_tools(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as directory:
            run_dir = Path(directory)
            turn = run_dir / "turns" / "01"
            turn.mkdir(parents=True)
            events = [
                {
                    "type": "item.completed",
                    "item": {
                        "type": "agent_message",
                        "text": "I did not visit https://example.test/solution",
                    },
                },
                {
                    "type": "item.completed",
                    "item": {
                        "type": "command_execution",
                        "command": "python -m unittest",
                        "aggregated_output": "docs mention https://example.test",
                    },
                },
                {
                    "type": "assistant",
                    "message": {
                        "content": [
                            {
                                "type": "tool_use",
                                "name": "WebSearch",
                                "input": {"query": "external answer"},
                            }
                        ]
                    },
                },
            ]
            (turn / "events.jsonl").write_text(
                "\n".join(json.dumps(event) for event in events) + "\n",
                encoding="utf-8",
            )

            audit = native.integrity_audit(run_dir)

            self.assertFalse(audit["valid"])
            self.assertEqual(len(audit["findings"]), 1)
            self.assertEqual(audit["findings"][0]["kind"], "external_tool")

    def test_record_quota_can_only_tighten_an_active_cap(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            run_dir = Path(directory)
            state_path = run_dir / native.STATE_FILE
            native.dump_json(
                state_path,
                {
                    "turns": [],
                    "quota_baseline_percent": 36.0,
                    "quota_cap_delta_points": 10.0,
                    "quota_observations": [],
                },
            )
            args = argparse.Namespace(
                run_dir=run_dir,
                weekly=37.0,
                source="test",
                external_observed_at=native.utc_now(),
                quota_cap=5.0,
            )

            with contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(native.record_quota(args), 0)
            state = native.load_json(state_path)
            self.assertEqual(state["quota_cap_delta_points"], 5.0)

            args.weekly = 41.0
            args.quota_cap = None
            with contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(native.record_quota(args), 0)
            state = native.load_json(state_path)
            self.assertIn("quota_cap_reached", state)
            self.assertFalse(native.quota_is_fresh(state, 900)[0])
            self.assertTrue(
                native.quota_is_fresh(state, 900, for_paid_call=False)[0]
            )

            args.quota_cap = 6.0
            with self.assertRaisesRegex(ValueError, "refusing to loosen"):
                native.record_quota(args)

            args.quota_cap = None
            args.weekly = 40.0
            args.external_observed_at = native.utc_now()
            with self.assertRaisesRegex(ValueError, "decreased"):
                native.record_quota(args)
            self.assertEqual(
                len(native.load_json(state_path)["quota_observations"]),
                2,
                "a rejected observation must not corrupt the retained ledger",
            )

    def test_api_and_test_preservation_detect_real_signature_and_skip_changes(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            reservations = root / "reservations"
            tests = root / "tests"
            reservations.mkdir()
            tests.mkdir()
            module = reservations / "service.py"
            module.write_text(
                "class Service:\n"
                "    async def reserve(self, sku: str, *, count: int = 1) -> bool:\n"
                "        return True\n",
                encoding="utf-8",
            )
            test_module = tests / "test_service.py"
            test_module.write_text(
                "import unittest\n"
                "class ServiceTests(unittest.TestCase):\n"
                "    def test_reserve(self):\n"
                "        self.assertTrue(True)\n",
                encoding="utf-8",
            )
            signatures = native.public_signatures(root)
            original = native.original_tests(root)

            module.write_text(
                "class Service:\n"
                "    def reserve(self, sku: str, count: int = 1) -> bool:\n"
                "        return True\n",
                encoding="utf-8",
            )
            test_module.write_text(
                "import unittest\n"
                "@unittest.skip('weakened')\n"
                "class ServiceTests(unittest.TestCase):\n"
                "    def test_reserve(self):\n"
                "        self.assertTrue(True)\n",
                encoding="utf-8",
            )

            self.assertNotEqual(signatures, native.public_signatures(root))
            self.assertNotEqual(original, native.original_tests(root))

    def test_preserved_test_contract_detects_setup_weakening(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            tests = root / "tests"
            tests.mkdir()
            path = tests / "test_service.py"
            path.write_text(
                "import unittest\n"
                "class ServiceTests(unittest.TestCase):\n"
                "    def setUp(self):\n"
                "        self.stock = 1\n"
                "    def test_reserve(self):\n"
                "        self.assertEqual(self.stock, 1)\n",
                encoding="utf-8",
            )
            original = native.preserved_test_contract(root)

            path.write_text(
                "import unittest\n"
                "class ServiceTests(unittest.TestCase):\n"
                "    def setUp(self):\n"
                "        self.stock = 999\n"
                "    def test_reserve(self):\n"
                "        self.assertEqual(self.stock, 1)\n",
                encoding="utf-8",
            )

            self.assertNotEqual(original, native.preserved_test_contract(root))

    def test_missing_token_telemetry_never_aggregates_to_zero(self) -> None:
        turns = [
            {"agent": {"tokens": native.token_metrics(100, 80, 10)}},
            {"agent": {"tokens": native.empty_tokens()}},
        ]

        totals = native.aggregate_tokens(turns)

        self.assertIsNone(totals["total_tokens"])
        self.assertFalse(totals["complete"])
        self.assertEqual(totals["turns_reported"], 1)

    def test_codex_cumulative_usage_is_deltad_before_aggregation(self) -> None:
        turns = [
            {
                "agent": {
                    "tokens": native.token_metrics(1000, 800, 100),
                }
            },
            {
                "agent": {
                    "tokens": native.token_metrics(1600, 1300, 180),
                }
            },
            {
                "agent": {
                    "tokens": native.token_metrics(2200, 1800, 250),
                }
            },
        ]

        self.assertTrue(native.normalize_codex_turn_tokens(turns))
        self.assertFalse(native.normalize_codex_turn_tokens(turns))
        self.assertEqual(
            [turn["agent"]["tokens"]["total_tokens"] for turn in turns],
            [1100, 680, 670],
        )
        self.assertEqual(
            native.aggregate_tokens(turns)["total_tokens"],
            2450,
            "deltas must telescope to the final cumulative snapshot, not sum snapshots",
        )
        self.assertEqual(
            turns[-1]["agent"]["cumulative_tokens"]["total_tokens"], 2450
        )
        turns[-1]["agent"]["cumulative_tokens"] = native.token_metrics(
            900, 700, 90
        )
        self.assertTrue(native.normalize_codex_turn_tokens(turns))
        self.assertIsNone(turns[-1]["agent"]["tokens"]["total_tokens"])
        self.assertIn(
            "decreased", turns[-1]["agent"]["token_accounting_error"]
        )

        decreasing = [
            {"agent": {"tokens": native.token_metrics(100, 80, 10)}},
            {"agent": {"tokens": native.token_metrics(90, 70, 9)}},
        ]
        native.normalize_codex_turn_tokens(decreasing)
        self.assertIsNone(decreasing[-1]["agent"]["tokens"]["total_tokens"])
        self.assertIn(
            "decreased", decreasing[-1]["agent"]["token_accounting_error"]
        )

    def test_zero_event_preflight_failure_is_preserved_but_does_not_block_retry(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as directory:
            turn_dir = Path(directory) / "turns" / "02"
            turn_dir.mkdir(parents=True)
            (turn_dir / "stderr.log").write_text(
                "error: unexpected argument '--sandbox' found\n\n"
                "Usage: codex exec resume --json [SESSION_ID] [PROMPT]\n",
                encoding="utf-8",
            )
            failure = {
                "turn": 2,
                "success": False,
                "turn_dir": str(turn_dir),
                "agent": {
                    "event_count": 0,
                    "session_id": None,
                    "tokens": native.empty_tokens(),
                },
                "process": {
                    "exit_code": 2,
                    "timed_out": False,
                    "wall_seconds": 0.016,
                },
            }
            state = {
                "next_turn": 1,
                "turns": [{"turn": 1, "success": True}, failure],
                "preflight_failures": [],
                "failed_turn": 2,
            }

            self.assertTrue(native.recover_preflight_failures(state))
            self.assertEqual(state["next_turn"], 1)
            self.assertEqual([turn["turn"] for turn in state["turns"]], [1])
            self.assertEqual(state["preflight_failures"], [failure])
            self.assertNotIn("failed_turn", state)

            (turn_dir / "stderr.log").write_text(
                "transport closed before the first event\n", encoding="utf-8"
            )
            self.assertFalse(
                native.is_preflight_failure(failure),
                "a zero-event provider/transport failure must never be assumed free",
            )

    def test_patch_capture_excludes_harness_noise_but_retains_source_changes(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            workspace = root / "workspace"
            run_dir = root / "run"
            workspace.mkdir()
            run_dir.mkdir()
            native.git(workspace, "init", "-q")
            native.git(workspace, "config", "user.email", "test@local")
            native.git(workspace, "config", "user.name", "Test")
            (workspace / "source.py").write_text("old\n", encoding="utf-8")
            native.git(workspace, "add", "-A")
            native.git(workspace, "commit", "-qm", "base")
            baseline = native.tiny_command(("git", "rev-parse", "HEAD"), cwd=workspace)

            (workspace / "source.py").write_text("new\n", encoding="utf-8")
            (workspace / "new_test.py").write_text("assert True\n", encoding="utf-8")
            checkpoint = workspace / ".forge" / "checkpoints"
            checkpoint.mkdir(parents=True)
            (checkpoint / "state.blob").write_text("generated\n", encoding="utf-8")
            cache = workspace / "pkg" / "__pycache__"
            cache.mkdir(parents=True)
            (cache / "module.pyc").write_bytes(b"generated")

            result = native.capture_patch(workspace, run_dir, baseline)
            patch = (run_dir / "changes.patch").read_text(encoding="utf-8")

            self.assertTrue(result["diff_check"])
            self.assertIn("source.py", patch)
            self.assertIn("new_test.py", patch)
            self.assertNotIn(".forge", patch)
            self.assertNotIn("__pycache__", patch)
            self.assertTrue(
                all(".forge" not in row and "__pycache__" not in row for row in result["status"])
            )
            self.assertEqual(
                (run_dir / "git-status.txt").read_text(encoding="utf-8").splitlines(),
                result["status"],
            )

    def test_auth_failure_recovery_requires_zero_tokens_and_restored_login(
        self,
    ) -> None:
        failure = {
            "success": False,
            "turn_dir": "/run/turns/01",
            "agent": {
                "event_count": 8,
                "provider_calls": 1,
                "result": "Failed to authenticate: OAuth session expired",
                "tokens": native.token_metrics(0, 0, 0),
            },
            "process": {"exit_code": 1, "timed_out": False, "wall_seconds": 1.4},
        }
        self.assertTrue(native.is_zero_token_auth_failure(failure))
        paid = {
            **failure,
            "agent": {
                **failure["agent"],
                "tokens": native.token_metrics(10, 0, 1),
            },
        }
        self.assertFalse(native.is_zero_token_auth_failure(paid))

        with tempfile.TemporaryDirectory() as directory:
            run_dir = Path(directory)
            native.dump_json(
                run_dir / native.STATE_FILE,
                {
                    "provider": "claude",
                    "next_turn": 0,
                    "paid_failure": failure,
                    "authentication_failures": [],
                },
            )
            with mock.patch.object(native, "claude_authenticated", return_value=False):
                with self.assertRaisesRegex(RuntimeError, "not restored"):
                    native.recover_auth(argparse.Namespace(run_dir=run_dir))
            with (
                mock.patch.object(native, "claude_authenticated", return_value=True),
                contextlib.redirect_stdout(io.StringIO()),
            ):
                self.assertEqual(
                    native.recover_auth(
                        argparse.Namespace(
                            run_dir=run_dir,
                            expected_resolved_model="claude-opus-5[1m]",
                        )
                    ),
                    0,
                )
            state = native.load_json(run_dir / native.STATE_FILE)
            self.assertNotIn("paid_failure", state)
            self.assertEqual(
                state["expected_resolved_model"], "claude-opus-5[1m]"
            )
            self.assertEqual(state["authentication_failures"], [failure])

    def test_logged_out_claude_is_denied_before_a_provider_process_starts(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as directory:
            run_dir = Path(directory)
            native.dump_json(
                run_dir / native.STATE_FILE,
                {
                    "provider": "claude",
                    "cli_version": "2.1.220 (Claude Code)",
                    "next_turn": 0,
                    "turns": [],
                    "preflight_failures": [],
                },
            )
            with (
                mock.patch.object(
                    native,
                    "current_cli_version",
                    return_value="2.1.220 (Claude Code)",
                ),
                mock.patch.object(native, "quota_is_fresh", return_value=(True, "")),
                mock.patch.object(
                    native, "claude_authenticated", return_value=False
                ),
                mock.patch.object(native, "run_capture") as capture,
                self.assertRaisesRegex(RuntimeError, "Claude CLI is logged out"),
            ):
                native.run_turn(
                    argparse.Namespace(
                        run_dir=run_dir,
                        timeout=10,
                        quota_max_age=900,
                    )
                )
            capture.assert_not_called()

            with (
                mock.patch.object(
                    native,
                    "current_cli_version",
                    return_value="2.1.221 (Claude Code)",
                ),
                mock.patch.object(native, "run_capture") as capture,
                self.assertRaisesRegex(RuntimeError, "CLI version changed"),
            ):
                native.run_turn(
                    argparse.Namespace(
                        run_dir=run_dir,
                        timeout=10,
                        quota_max_age=900,
                    )
                )
            capture.assert_not_called()

    def test_six_mocked_turns_preserve_one_session_and_never_duplicate_a_call(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as directory:
            out_root = Path(directory)
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                native.prepare(
                    argparse.Namespace(
                        provider="codex",
                        model="gpt-5.6-sol",
                        effort="high",
                        scenario=native.DEFAULT_SCENARIO,
                        out_root=out_root,
                        quota_baseline=36.0,
                        quota_cap=10.0,
                    )
                )
            run_dir = next(out_root.iterdir())
            prepared_state = native.load_json(run_dir / native.STATE_FILE)
            prepared_workspace = Path(prepared_state["workspace"])
            self.assertEqual(
                prepared_state["synthetic_base_tree"],
                native.tiny_command(
                    (
                        "git",
                        "rev-parse",
                        f"{prepared_state['synthetic_base_commit']}^{{tree}}",
                    ),
                    cwd=prepared_workspace,
                ),
            )
            self.assertFalse(
                any(prepared_workspace.rglob("*.pyc")),
                "generated interpreter caches must not enter the synthetic base",
            )

            def fake_capture(argv, *, cwd, stdout_path, stderr_path, timeout):
                stdout_path.write_text("{}\n", encoding="utf-8")
                stderr_path.write_text("", encoding="utf-8")
                return {
                    "started_at": native.utc_now(),
                    "ended_at": native.utc_now(),
                    "wall_seconds": 1.0,
                    "exit_code": 0,
                    "timed_out": False,
                    "argv": list(argv),
                }

            def fake_summary(_path):
                return {
                    "session_id": "one-session",
                    "completed": True,
                    "result": "done",
                    "event_count": 1,
                    "malformed_event_lines": 0,
                    "tool_calls": 1,
                    "tokens": native.token_metrics(100, 80, 10),
                }

            with (
                mock.patch.object(native, "run_capture", side_effect=fake_capture),
                mock.patch.object(native, "summarize_codex", side_effect=fake_summary),
                mock.patch.object(
                    native,
                    "codex_rollout_context",
                    return_value={
                        "resolved_model": "gpt-5.6-sol",
                        "resolved_effort": "high",
                    },
                ),
            ):
                for turn in range(6):
                    with contextlib.redirect_stdout(output):
                        native.record_quota(
                            argparse.Namespace(
                                run_dir=run_dir,
                                weekly=36.0,
                                source="mock",
                                external_observed_at=native.utc_now(),
                            )
                        )
                        result = native.run_turn(
                            argparse.Namespace(
                                run_dir=run_dir,
                                timeout=10,
                                quota_max_age=900,
                            )
                        )
                    self.assertEqual(result, 0, f"turn {turn + 1}")

            state = native.load_json(run_dir / native.STATE_FILE)
            self.assertEqual(state["next_turn"], 6)
            self.assertEqual(len(state["turns"]), 6)
            self.assertTrue(all(turn["same_session"] for turn in state["turns"]))
            self.assertEqual(
                [turn["turn"] for turn in state["turns"]],
                [1, 2, 3, 4, 5, 6],
            )


if __name__ == "__main__":
    unittest.main()
