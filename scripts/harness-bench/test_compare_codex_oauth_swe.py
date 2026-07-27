#!/usr/bin/env python3

from __future__ import annotations

import subprocess
import tempfile
import unittest
from pathlib import Path

import compare_codex_oauth as bench
import compare_codex_oauth_swe as swe


class HistoryIsolationTests(unittest.TestCase):
    def test_prepare_repo_removes_future_history_and_remotes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            workspace = root / "owner__repo-1"
            workspace.mkdir()
            self.run_git(workspace, "init", "--quiet")
            self.run_git(workspace, "config", "user.email", "test@forge.invalid")
            self.run_git(workspace, "config", "user.name", "Forge Test")
            (workspace / "value.txt").write_text("base\n", encoding="utf-8")
            self.run_git(workspace, "add", "value.txt")
            self.run_git(workspace, "commit", "--quiet", "-m", "base")
            base = self.git(workspace, "rev-parse", "HEAD")
            (workspace / "value.txt").write_text("future fix\n", encoding="utf-8")
            self.run_git(workspace, "commit", "--quiet", "-am", "future gold fix")
            self.run_git(
                workspace,
                "remote",
                "add",
                "origin",
                "https://example.invalid/owner/repo.git",
            )

            prepared = swe.prepare_repo(
                {
                    "instance_id": "owner__repo-1",
                    "repo": "owner/repo",
                    "base_commit": base,
                },
                root,
            )

            self.assertEqual(prepared, workspace)
            self.assertEqual(self.git(workspace, "rev-list", "--all", "--count"), "1")
            self.assertNotIn("future gold fix", self.git(workspace, "log", "--all"))
            self.assertEqual(self.git(workspace, "remote"), "")
            self.assertEqual(
                (workspace / "value.txt").read_text(encoding="utf-8"),
                "base\n",
            )
            self.assertEqual(
                (workspace / ".git" / swe.BENCHMARK_ORIGIN_MARKER)
                .read_text(encoding="utf-8")
                .strip(),
                base,
            )

    def test_capture_patch_includes_committed_agent_changes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            workspace = root / "workspace"
            trial = root / "trial"
            workspace.mkdir()
            trial.mkdir()
            self.run_git(workspace, "init", "--quiet")
            self.run_git(workspace, "config", "user.email", "test@forge.invalid")
            self.run_git(workspace, "config", "user.name", "Forge Test")
            (workspace / "value.txt").write_text("base\n", encoding="utf-8")
            self.run_git(workspace, "add", "value.txt")
            self.run_git(workspace, "commit", "--quiet", "-m", "base")
            base = self.git(workspace, "rev-parse", "HEAD")
            self.run_git(workspace, "update-ref", swe.BENCHMARK_BASE_REF, base)

            (workspace / "value.txt").write_text("agent change\n", encoding="utf-8")
            self.run_git(workspace, "commit", "--quiet", "-am", "agent commit")

            summary = bench.capture_patch(
                workspace,
                trial,
                base_ref=swe.BENCHMARK_BASE_REF,
            )
            patch = (trial / "changes.patch").read_text(encoding="utf-8")

            self.assertIn("-base", patch)
            self.assertIn("+agent change", patch)
            self.assertIn("M\tvalue.txt", summary["status"])

    def test_benchmark_prompt_forbids_external_solution_leaks(self) -> None:
        prompt = swe.benchmark_prompt("Fix the issue.")
        self.assertIn("Do not access the network", prompt)
        self.assertIn("Do not search Git history", prompt)
        self.assertTrue(prompt.endswith("Fix the issue."))

    def test_forge_state_is_scoped_to_one_trial(self) -> None:
        trial_dir = Path("/benchmark/run/trials/001__model__task__forge")
        self.assertEqual(
            swe.trial_forge_db(trial_dir),
            trial_dir / "forge-benchmark.db",
        )

    def test_effective_quota_is_conservative(self) -> None:
        summaries = [
            {"quota": {"used_percent": 31}},
            {"quota": {"used_percent": 29}},
        ]
        self.assertEqual(swe.effective_quota(summaries, 30), 31)
        self.assertEqual(swe.effective_quota([], 30), 30)

    def test_validate_resume_rejects_changed_suite_inputs(self) -> None:
        with self.assertRaisesRegex(ValueError, "resume manifest mismatch"):
            swe.validate_resume(
                {"dataset_sha256": "old"},
                {"dataset_sha256": "new"},
            )

    def test_native_codex_can_use_regular_high_effort(self) -> None:
        argv = bench.raw_codex_argv(
            "gpt-5.6-sol",
            Path("/tmp/workspace"),
            "Fix it.",
            "native",
            "high",
        )
        self.assertIn('model_reasoning_effort="high"', argv)
        self.assertNotIn('model_reasoning_effort="xhigh"', argv)

    def test_forge_codex_can_use_regular_high_effort(self) -> None:
        environment = bench.forge_environment(
            Path("/tmp/forge-benchmark.db"),
            "high",
        )
        self.assertEqual(environment["FORGE_MESH__DEFAULT_EFFORT"], "high")
        self.assertEqual(environment["FORGE_MESH__FAILOVER"], "true")
        self.assertEqual(environment["FORGE_MESH__PIN_FAILOVER"], "false")

    @staticmethod
    def run_git(workspace: Path, *args: str) -> None:
        subprocess.run(
            ("git", *args),
            cwd=workspace,
            check=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )

    @staticmethod
    def git(workspace: Path, *args: str) -> str:
        return subprocess.run(
            ("git", *args),
            cwd=workspace,
            check=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        ).stdout.strip()


if __name__ == "__main__":
    unittest.main()
