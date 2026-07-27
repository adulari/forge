from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import audit_benchmark_cells as audit


class BenchmarkCellAuditTests(unittest.TestCase):
    def test_dataset_evidence_verifies_hash_and_index(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            dataset = Path(directory) / "dataset.jsonl"
            payload = (
                "\n"
                + json.dumps(
                    {
                        "instance_id": "owner__repo-1",
                        "problem_statement": "Fix it.",
                    }
                )
                + "\n"
            ).encode()
            dataset.write_bytes(payload)

            result = audit.dataset_evidence(
                {
                    "dataset": str(dataset),
                    "dataset_sha256": audit.sha256_bytes(payload),
                }
            )

        self.assertTrue(result["metadata_matches"])
        self.assertEqual(result["instances"]["owner__repo-1"]["_dataset_index"], 0)

    def test_process_identity_covers_pinned_and_native_models(self) -> None:
        cases = (
            (
                "forge",
                "gpt-5.6-terra",
                ["forge", "run", "--model", "codex-oauth::gpt-5.6-terra"],
            ),
            (
                "forge",
                "opus[1m]",
                ["forge", "run", "--model", "claude-cli::opus[1m]"],
            ),
            (
                "raw-codex",
                "gpt-5.6-sol",
                [
                    "codex",
                    "exec",
                    "-m",
                    "gpt-5.6-sol",
                    "-c",
                    'model_reasoning_effort="high"',
                ],
            ),
            (
                "raw-claude",
                "sonnet",
                ["claude", "--effort", "high", "--model", "sonnet"],
            ),
        )

        for arm, model, argv in cases:
            with self.subTest(arm=arm, model=model):
                result = audit.expected_process_identity(
                    arm=arm,
                    model=model,
                    argv=argv,
                )
                self.assertTrue(result["model_matches"])
                self.assertTrue(result["effort_matches"])

    def test_mesh_process_must_remain_unpinned(self) -> None:
        clean = audit.expected_process_identity(
            arm="forge-mesh-auto",
            model=None,
            argv=["forge", "run", "--mode", "bypass"],
        )
        pinned = audit.expected_process_identity(
            arm="forge-mesh-auto",
            model=None,
            argv=["forge", "run", "--model", "codex-oauth::gpt-5.6-terra"],
        )

        self.assertTrue(clean["unpinned_mesh"])
        self.assertFalse(pinned["unpinned_mesh"])

    def test_patch_capture_requires_matching_nonempty_metadata(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            summary = Path(directory) / "summary.json"
            patch = summary.with_name("changes.patch")
            patch.write_bytes(b"diff --git a/a b/a\n")
            payload = patch.read_bytes()
            result = audit.patch_capture(
                summary,
                {
                    "patch": {
                        "patch_bytes": len(payload),
                        "patch_sha256": audit.sha256_bytes(payload),
                    }
                },
            )

        self.assertTrue(result["metadata_matches"])

    def test_url_literal_in_edit_is_not_a_network_lookup(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            events = Path(directory) / "events.jsonl"
            events.write_text(
                json.dumps(
                    {
                        "message": {
                            "content": [
                                {
                                    "type": "tool_use",
                                    "name": "mcp__forge__edit_file",
                                    "input": {"new": "# https://placeholder"},
                                }
                            ]
                        }
                    }
                )
                + "\n",
                encoding="utf-8",
            )

            result = audit.audit_events(events)

        self.assertEqual(result["findings"], [])

    def test_shell_network_command_is_an_integrity_violation(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            events = Path(directory) / "events.jsonl"
            events.write_text(
                json.dumps(
                    {
                        "message": {
                            "content": [
                                {
                                    "type": "tool_use",
                                    "name": "mcp__forge__shell",
                                    "input": {"command": "curl https://example.invalid"},
                                }
                            ]
                        }
                    }
                )
                + "\n",
                encoding="utf-8",
            )

            result = audit.audit_events(events)

        self.assertEqual(
            {finding["kind"] for finding in result["findings"]},
            {"network_command"},
        )

    def test_git_history_command_is_an_integrity_violation(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            events = Path(directory) / "events.jsonl"
            events.write_text(
                json.dumps(
                    {
                        "item": {
                            "type": "command_execution",
                            "command": "git log --oneline",
                        }
                    }
                )
                + "\n",
                encoding="utf-8",
            )

            result = audit.audit_events(events)

        self.assertEqual(
            {finding["kind"] for finding in result["findings"]},
            {"git_history_or_remote"},
        )

    def test_git_clone_is_an_integrity_violation(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            events = Path(directory) / "events.jsonl"
            events.write_text(
                json.dumps(
                    {
                        "item": {
                            "type": "command_execution",
                            "command": "git clone https://example.invalid/solution.git",
                        }
                    }
                )
                + "\n",
                encoding="utf-8",
            )

            result = audit.audit_events(events)

        self.assertEqual(
            {finding["kind"] for finding in result["findings"]},
            {"git_history_or_remote"},
        )

    def test_ssh_is_a_network_integrity_violation(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            events = Path(directory) / "events.jsonl"
            events.write_text(
                json.dumps(
                    {
                        "item": {
                            "type": "command_execution",
                            "command": "ssh example.invalid uname -a",
                        }
                    }
                )
                + "\n",
                encoding="utf-8",
            )

            result = audit.audit_events(events)

        self.assertEqual(
            {finding["kind"] for finding in result["findings"]},
            {"network_command"},
        )


if __name__ == "__main__":
    unittest.main()
