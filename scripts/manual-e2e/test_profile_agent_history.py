import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))

import profile_agent_history as profile


class HistoryProfileTests(unittest.TestCase):
    def test_project_bucket_distinguishes_benchmark_fixtures(self) -> None:
        self.assertEqual(
            profile.project_bucket("/tmp/forge-harness-bench/cell"),
            "forge_benchmark_fixture",
        )
        self.assertEqual(profile.project_bucket("/workspace/forge"), "forge")

    def test_codex_profile_counts_only_real_user_events(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "codex.jsonl"
            records = [
                {
                    "type": "session_meta",
                    "timestamp": "2026-07-27T10:00:00Z",
                    "payload": {"cwd": "/workspace/forge"},
                },
                {
                    "type": "event_msg",
                    "timestamp": "2026-07-27T10:01:00Z",
                    "payload": {
                        "type": "user_message",
                        "message": "continue the benchmark and fix the regression",
                    },
                },
                {
                    "type": "response_item",
                    "timestamp": "2026-07-27T10:02:00Z",
                    "payload": {"type": "function_call", "name": "exec_command"},
                },
                {
                    "type": "response_item",
                    "timestamp": "2026-07-27T10:02:01Z",
                    "payload": {"type": "function_call_output"},
                },
                {
                    "type": "compacted",
                    "timestamp": "2026-07-27T10:03:00Z",
                    "payload": {},
                },
                {
                    "type": "event_msg",
                    "timestamp": "2026-07-27T10:04:00Z",
                    "payload": {"type": "turn_aborted"},
                },
            ]
            path.write_text(
                "\n".join(json.dumps(record) for record in records) + "\n{broken",
                encoding="utf-8",
            )

            metrics = profile.profile_codex(path)

            self.assertEqual(metrics.project, "forge")
            self.assertEqual(metrics.user_turns, 1)
            self.assertEqual(metrics.tool_calls, 1)
            self.assertEqual(metrics.tool_results, 1)
            self.assertEqual(metrics.compactions, 1)
            self.assertEqual(metrics.aborted_turns, 1)
            self.assertEqual(metrics.malformed_records, 1)
            self.assertEqual(metrics.reprompts["continuation_persistence"], 1)
            self.assertEqual(metrics.categories["benchmark_performance"], 1)

    def test_claude_profile_excludes_tool_results_from_user_turns(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "claude.jsonl"
            records = [
                {
                    "type": "user",
                    "timestamp": "2026-07-27T10:00:00Z",
                    "cwd": "/workspace/forge",
                    "message": {"content": "audit the routing design and continue"},
                },
                {
                    "type": "user",
                    "timestamp": "2026-07-27T10:00:01Z",
                    "cwd": "/workspace/forge",
                    "message": {
                        "content": [
                            {
                                "type": "tool_result",
                                "tool_use_id": "call-1",
                                "content": "ok",
                            }
                        ]
                    },
                },
                {
                    "type": "assistant",
                    "timestamp": "2026-07-27T10:00:02Z",
                    "cwd": "/workspace/forge",
                    "message": {
                        "model": "claude-sonnet-5",
                        "content": [
                            {
                                "type": "tool_use",
                                "id": "call-1",
                                "name": "Bash",
                                "input": {},
                            }
                        ],
                    },
                },
                {
                    "type": "system",
                    "subtype": "compact_boundary",
                    "timestamp": "2026-07-27T10:00:03Z",
                    "cwd": "/workspace/forge",
                },
            ]
            path.write_text(
                "\n".join(json.dumps(record) for record in records),
                encoding="utf-8",
            )

            metrics = profile.profile_claude(path)

            self.assertEqual(metrics.user_turns, 1)
            self.assertEqual(metrics.tool_results, 1)
            self.assertEqual(metrics.tool_calls, 1)
            self.assertEqual(metrics.tools["Bash"], 1)
            self.assertEqual(metrics.compactions, 1)
            self.assertEqual(metrics.models["claude-sonnet-5"], 1)
            self.assertEqual(metrics.categories["review_audit"], 1)


if __name__ == "__main__":
    unittest.main()
