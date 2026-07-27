#!/usr/bin/env python3

from __future__ import annotations

import json
import sqlite3
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import compare_mesh_swe as mesh


def write_analysis(
    path: Path,
    *,
    family: str,
    count: int,
) -> None:
    raw_prefix = "raw_codex" if family == "codex" else "raw_claude"
    rows = []
    for index in range(count):
        row = {
            "model": f"{family}-model-{index % 3}",
            "instance_id": f"repo__issue-{index}",
            "pair_id": f"{family}-{index}",
            "dataset_index": index,
            "difficulty": "<15 min fix",
            "repo": "owner/repo",
            f"{raw_prefix}_resolved": index % 2 == 0,
            f"{raw_prefix}_wall_seconds": 10 + index,
            f"{raw_prefix}_total_tokens": 100 + index,
            f"{raw_prefix}_cache_adjusted_tokens_025": 50 + index,
        }
        rows.append(row)
    path.write_text(json.dumps({"pairs": rows}), encoding="utf-8")


class MeshBenchmarkTests(unittest.TestCase):
    def test_baseline_cells_accept_quota_sized_clean_studies(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            codex = root / "codex.json"
            claude = root / "claude.json"
            write_analysis(codex, family="codex", count=6)
            write_analysis(claude, family="claude", count=4)

            cells = mesh.baseline_cells(codex, claude)

        self.assertEqual(len(cells), 10)
        self.assertEqual(sum(cell["family"] == "codex" for cell in cells), 6)
        self.assertEqual(sum(cell["family"] == "claude" for cell in cells), 4)
        self.assertEqual(cells[0]["baseline"]["cache_adjusted_tokens_025"], 50.0)

    def test_baseline_cells_require_both_native_families(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            codex = root / "codex.json"
            claude = root / "claude.json"
            write_analysis(codex, family="codex", count=2)
            claude.write_text(json.dumps({"pairs": []}), encoding="utf-8")

            with self.assertRaisesRegex(ValueError, "missing non-empty pairs"):
                mesh.baseline_cells(codex, claude)

    def test_plan_is_seeded_and_runs_each_unique_task_once(self) -> None:
        cells = [
            {
                "family": "codex",
                "comparator_model": f"model-{model}",
                "instance_id": f"issue-{instance}",
                "baseline": {"resolved": True},
            }
            for instance in range(5)
            for model in range(3)
        ]

        first = mesh.plan_trials(cells, 42)
        second = mesh.plan_trials(cells, 42)

        self.assertEqual(
            [trial.cell_id for trial in first], [trial.cell_id for trial in second]
        )
        self.assertEqual(len({trial.cell_id for trial in first}), 5)
        self.assertEqual(len(first), 5)
        self.assertTrue(all(len(trial.baselines) == 3 for trial in first))

    def test_plan_prioritizes_cross_family_coverage_then_difficulty(self) -> None:
        cells = [
            {
                "family": "codex",
                "comparator_model": "codex-model",
                "instance_id": "codex-easy",
                "difficulty": "<15 min fix",
                "baseline": {"resolved": True},
            },
            {
                "family": "codex",
                "comparator_model": "codex-model",
                "instance_id": "shared-hard",
                "difficulty": "1-4 hours",
                "baseline": {"resolved": True},
            },
            {
                "family": "claude",
                "comparator_model": "claude-model",
                "instance_id": "shared-hard",
                "difficulty": "1-4 hours",
                "baseline": {"resolved": True},
            },
            {
                "family": "codex",
                "comparator_model": "codex-model",
                "instance_id": "shared-easy",
                "difficulty": "<15 min fix",
                "baseline": {"resolved": True},
            },
            {
                "family": "claude",
                "comparator_model": "claude-model",
                "instance_id": "shared-easy",
                "difficulty": "<15 min fix",
                "baseline": {"resolved": True},
            },
        ]

        trials = mesh.plan_trials(cells, 42)

        self.assertEqual(
            [trial.instance_id for trial in trials],
            ["shared-easy", "shared-hard", "codex-easy"],
        )

    def test_mesh_argv_is_unpinned_and_has_no_effort_override(self) -> None:
        argv = mesh.mesh_argv(Path("/tmp/forge"), "fix it")
        self.assertNotIn("--model", argv)
        self.assertNotIn("--effort", argv)
        self.assertEqual(argv[-1], "fix it")

    def test_mesh_environment_removes_inherited_mesh_overrides(self) -> None:
        with mock.patch.dict(
            "os.environ",
            {
                "FORGE_MESH__DEFAULT_EFFORT": "xhigh",
                "FORGE_MESH__AUTO_DISCOVER": "false",
                "UNRELATED": "kept",
            },
            clear=True,
        ):
            environment = mesh.mesh_environment(
                Path("/tmp/bench.db"),
                Path("/tmp/trial"),
            )

        self.assertNotIn("FORGE_MESH__DEFAULT_EFFORT", environment)
        self.assertNotIn("FORGE_MESH__AUTO_DISCOVER", environment)
        self.assertEqual(environment["UNRELATED"], "kept")
        self.assertEqual(environment["FORGE_DB"], "/tmp/bench.db")

    def test_selected_model_reads_actual_routing_event(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            events = Path(directory) / "events.jsonl"
            events.write_text(
                "\n".join(
                    (
                        json.dumps(
                            {
                                "type": "system",
                                "subtype": "init",
                                "model": "not-the-selected-route",
                            }
                        ),
                        json.dumps(
                            {
                                "type": "system",
                                "subtype": "routing",
                                "model": "codex-oauth::gpt-5.6-terra",
                            }
                        ),
                    )
                )
                + "\n",
                encoding="utf-8",
            )

            self.assertEqual(
                mesh.selected_model(events),
                "codex-oauth::gpt-5.6-terra",
            )

    def test_mesh_patch_capture_includes_committed_changes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            workspace = root / "workspace"
            trial = root / "trial"
            workspace.mkdir()
            trial.mkdir()
            for args in (
                ("init", "--quiet"),
                ("config", "user.email", "test@forge.invalid"),
                ("config", "user.name", "Forge Test"),
            ):
                subprocess.run(
                    ("git", *args),
                    cwd=workspace,
                    check=True,
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                )
            (workspace / "value.txt").write_text("base\n", encoding="utf-8")
            subprocess.run(
                ("git", "add", "value.txt"),
                cwd=workspace,
                check=True,
                stdout=subprocess.DEVNULL,
            )
            subprocess.run(
                ("git", "commit", "--quiet", "-m", "base"),
                cwd=workspace,
                check=True,
            )
            base = subprocess.run(
                ("git", "rev-parse", "HEAD"),
                cwd=workspace,
                check=True,
                stdout=subprocess.PIPE,
                text=True,
            ).stdout.strip()
            subprocess.run(
                ("git", "update-ref", mesh.swe.BENCHMARK_BASE_REF, base),
                cwd=workspace,
                check=True,
            )
            (workspace / "value.txt").write_text("committed\n", encoding="utf-8")
            subprocess.run(
                ("git", "commit", "--quiet", "-am", "agent commit"),
                cwd=workspace,
                check=True,
            )

            summary = mesh.capture_mesh_patch(workspace, trial)
            patch = (trial / "changes.patch").read_text(encoding="utf-8")

            self.assertIn("+committed", patch)
            self.assertIn("M\tvalue.txt", summary["status"])

    def test_route_usage_recovers_model_and_provider_from_message(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            database = Path(directory) / "forge.db"
            with sqlite3.connect(database) as connection:
                connection.executescript(
                    """
                    CREATE TABLE session (
                        id TEXT PRIMARY KEY,
                        parent_session_id TEXT
                    );
                    CREATE TABLE message (
                        id TEXT PRIMARY KEY,
                        session_id TEXT,
                        model TEXT
                    );
                    CREATE TABLE usage (
                        message_id TEXT,
                        provider TEXT,
                        model TEXT,
                        input_tokens INTEGER,
                        cached_input_tokens INTEGER,
                        output_tokens INTEGER
                    );
                    INSERT INTO session VALUES ('root', NULL);
                    INSERT INTO session VALUES ('child', 'root');
                    INSERT INTO message
                        VALUES ('message-1', 'child', 'codex-oauth::gpt-5.6-terra');
                    INSERT INTO usage
                        VALUES ('message-1', NULL, NULL, 100, 40, 20);
                    """
                )

            usage = mesh.route_usage(database, "root")

        self.assertEqual(
            usage,
            [
                {
                    "provider": "codex-oauth",
                    "model": "codex-oauth::gpt-5.6-terra",
                    "provider_calls": 1,
                    "input_tokens": 100,
                    "cached_input_tokens": 40,
                    "output_tokens": 20,
                    "total_tokens": 120,
                    "cache_adjusted_tokens_025": 90.0,
                }
            ],
        )

    def test_effective_quota_is_conservative(self) -> None:
        summaries = [
            {
                "quota": {
                    "claude": {"used_percent": 24},
                    "codex": {"used_percent": 30},
                }
            }
        ]
        self.assertEqual(mesh.effective_quota(summaries, "claude", 25), 25)
        self.assertEqual(mesh.effective_quota(summaries, "codex", 26), 30)


if __name__ == "__main__":
    unittest.main()
