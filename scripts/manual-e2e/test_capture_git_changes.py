import shlex
import subprocess
import tempfile
import unittest
from pathlib import Path


HERE = Path(__file__).resolve().parent
CAPTURE_SCRIPT = HERE / "capture_git_changes.sh"


def git(workspace: Path, *args: str) -> str:
    result = subprocess.run(
        ["git", "-C", str(workspace), *args],
        check=True,
        capture_output=True,
        text=True,
    )
    return result.stdout.strip()


class CaptureGitChangesTests(unittest.TestCase):
    def test_captures_committed_and_untracked_edits_but_skips_ignored_runtime(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            workspace = root / "workspace"
            run_dir = root / "run"
            workspace.mkdir()
            run_dir.mkdir()

            git(workspace, "init", "--quiet")
            git(workspace, "config", "user.name", "Benchmark Test")
            git(workspace, "config", "user.email", "benchmark@example.invalid")
            (workspace / ".gitignore").write_text(".forge/\n", encoding="utf-8")
            (workspace / "tracked.txt").write_text("base\n", encoding="utf-8")
            git(workspace, "add", ".")
            git(workspace, "commit", "--quiet", "-m", "synthetic base")
            base = git(workspace, "rev-parse", "HEAD")

            (workspace / "tracked.txt").write_text("committed edit\n", encoding="utf-8")
            git(workspace, "add", "tracked.txt")
            git(workspace, "commit", "--quiet", "-m", "agent commit")
            (workspace / "new source.txt").write_text("uncommitted edit\n", encoding="utf-8")
            ignored = workspace / ".forge"
            ignored.mkdir()
            (ignored / "session.json").write_text("{}\n", encoding="utf-8")

            command = " ".join(
                [
                    f"source {shlex.quote(str(CAPTURE_SCRIPT))};",
                    "capture_git_changes",
                    shlex.quote(str(workspace)),
                    shlex.quote(base),
                    shlex.quote(str(run_dir)),
                    shlex.quote("."),
                    shlex.quote(":(exclude).forge/**"),
                ]
            )
            subprocess.run(["bash", "-c", command], check=True)

            patch = (run_dir / "changes.patch").read_text(encoding="utf-8")
            status = (run_dir / "git-status.txt").read_text(encoding="utf-8")
            self.assertIn("+committed edit", patch)
            self.assertIn("new source.txt", patch)
            self.assertIn("+uncommitted edit", patch)
            self.assertNotIn(".forge", patch)
            self.assertIn("new source.txt", status)
            self.assertNotIn(".forge", status)
            self.assertEqual(
                (run_dir / "git-diff-check.log").read_text(encoding="utf-8"),
                "",
            )


if __name__ == "__main__":
    unittest.main()
