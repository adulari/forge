from __future__ import annotations

import subprocess
import tempfile
import unittest
from pathlib import Path

import compare_codex_oauth as bench


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
