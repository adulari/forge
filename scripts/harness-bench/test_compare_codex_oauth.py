from __future__ import annotations

import sqlite3
import subprocess
import tempfile
import unittest
from unittest import mock
from pathlib import Path

import analyze_codex_oauth_swe as swe_analysis
import compare_codex_oauth as bench


class ForgeSessionTokenTests(unittest.TestCase):
    def test_rolls_up_descendant_sessions_without_unrelated_usage(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            db_path = Path(temp_dir) / "forge.db"
            with sqlite3.connect(db_path) as connection:
                connection.executescript(
                    """
                    CREATE TABLE session (
                        id TEXT PRIMARY KEY,
                        parent_session_id TEXT
                    );
                    CREATE TABLE message (
                        id TEXT PRIMARY KEY,
                        session_id TEXT NOT NULL
                    );
                    CREATE TABLE usage (
                        id TEXT PRIMARY KEY,
                        message_id TEXT NOT NULL,
                        input_tokens INTEGER NOT NULL,
                        cached_input_tokens INTEGER NOT NULL,
                        output_tokens INTEGER NOT NULL
                    );

                    INSERT INTO session VALUES ('root', NULL);
                    INSERT INTO session VALUES ('child', 'root');
                    INSERT INTO session VALUES ('grandchild', 'child');
                    INSERT INTO session VALUES ('unrelated', NULL);

                    INSERT INTO message VALUES ('m-root', 'root');
                    INSERT INTO message VALUES ('m-child', 'child');
                    INSERT INTO message VALUES ('m-grandchild', 'grandchild');
                    INSERT INTO message VALUES ('m-unrelated', 'unrelated');

                    INSERT INTO usage VALUES ('u-root', 'm-root', 100, 40, 10);
                    INSERT INTO usage VALUES ('u-child', 'm-child', 200, 80, 20);
                    INSERT INTO usage VALUES (
                        'u-grandchild', 'm-grandchild', 300, 120, 30
                    );
                    INSERT INTO usage VALUES (
                        'u-unrelated', 'm-unrelated', 1000, 400, 100
                    );
                    """
                )

            tokens = bench.forge_session_tokens(db_path, "root")

            self.assertIsNotNone(tokens)
            assert tokens is not None
            self.assertEqual(tokens["provider_calls"], 3)
            self.assertEqual(tokens["input_tokens"], 600)
            self.assertEqual(tokens["cached_input_tokens"], 240)
            self.assertEqual(tokens["uncached_input_tokens"], 360)
            self.assertEqual(tokens["output_tokens"], 60)
            self.assertEqual(tokens["total_tokens"], 660)

    def test_official_analysis_refreshes_forge_but_not_raw_tokens(self) -> None:
        summaries = [
            {
                "pair_id": "model__instance",
                "trial": {"arm": "forge"},
                "agent": {
                    "session_id": "root",
                    "tokens": {"total_tokens": 110},
                },
            },
            {
                "pair_id": "model__instance",
                "trial": {"arm": "raw-codex"},
                "agent": {
                    "session_id": "raw-thread",
                    "tokens": {"total_tokens": 220},
                },
            },
        ]
        rolled_up = {"provider_calls": 3, "total_tokens": 660}

        with mock.patch.object(
            swe_analysis.bench,
            "forge_session_tokens",
            return_value=rolled_up,
        ) as token_rollup:
            swe_analysis.refresh_forge_token_rollups(
                summaries, Path("/benchmark/forge.db")
            )

        token_rollup.assert_called_once_with(Path("/benchmark/forge.db"), "root")
        self.assertEqual(summaries[0]["agent"]["tokens"], rolled_up)
        self.assertEqual(summaries[1]["agent"]["tokens"]["total_tokens"], 220)


class PatchCaptureTests(unittest.TestCase):
    def test_capture_excludes_checkpoints_but_keeps_project_forge_files(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            workspace = root / "workspace"
            trial_dir = root / "trial"
            workspace.mkdir()
            trial_dir.mkdir()
            self.run_git(workspace, "init", "--quiet")
            self.run_git(workspace, "config", "user.email", "benchmark@test.invalid")
            self.run_git(workspace, "config", "user.name", "Benchmark Test")
            (workspace / "tracked.txt").write_text("before\n", encoding="utf-8")
            self.run_git(workspace, "add", "tracked.txt")
            self.run_git(workspace, "commit", "--quiet", "-m", "baseline")

            (workspace / "tracked.txt").write_text("after\n", encoding="utf-8")
            checkpoint = workspace / ".forge" / "checkpoints" / "session" / "2"
            checkpoint.mkdir(parents=True)
            (checkpoint / "0.blob").write_text("internal\n", encoding="utf-8")
            project_file = workspace / ".forge" / "workflows" / "project.js"
            project_file.parent.mkdir(parents=True)
            project_file.write_text("export default {};\n", encoding="utf-8")

            summary = bench.capture_patch(workspace, trial_dir)
            patch = (trial_dir / "changes.patch").read_text(encoding="utf-8")

            self.assertIn("tracked.txt", patch)
            self.assertIn(".forge/workflows/project.js", patch)
            self.assertNotIn(".forge/checkpoints", patch)
            self.assertTrue(
                all(".forge/checkpoints" not in line for line in summary["status"])
            )

    def test_local_excludes_are_appended_once(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            workspace = Path(temp_dir)
            self.run_git(workspace, "init", "--quiet")
            exclude_path = workspace / ".git" / "info" / "exclude"
            original = exclude_path.read_text(encoding="utf-8")

            bench.add_local_git_excludes(workspace, (".forge/checkpoints/", "*.pyc"))
            bench.add_local_git_excludes(workspace, (".forge/checkpoints/", "*.pyc"))
            updated = exclude_path.read_text(encoding="utf-8")

            self.assertTrue(updated.startswith(original))
            self.assertEqual(updated.splitlines().count(".forge/checkpoints/"), 1)
            self.assertEqual(updated.splitlines().count("*.pyc"), 1)

    @staticmethod
    def run_git(workspace: Path, *args: str) -> None:
        subprocess.run(
            ("git", *args),
            cwd=workspace,
            check=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )


if __name__ == "__main__":
    unittest.main()
