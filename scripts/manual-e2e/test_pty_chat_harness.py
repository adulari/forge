import json
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from pty_chat_harness import (
    prompt_persisted,
    session_state,
    surface_state,
    terminal_turn_failure,
    timeout_kind,
    wait_for_turn_permit,
)


class QuotaTurnGateTests(unittest.TestCase):
    def permit(self, *, codex: float = 12.0, claude: float = 29.0) -> dict[str, object]:
        return {
            "allow": True,
            "observed_at": "2026-07-28T12:00:00Z",
            "codex_weekly_percent": codex,
            "codex_hard_stop_percent": 22.0,
            "claude_weekly_percent": claude,
            "claude_hard_stop_percent": 32.0,
        }

    def test_permit_records_fresh_observation_and_wait_separately(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            gate_dir = Path(temp)
            (gate_dir / "permit-turn-02.json").write_text(
                json.dumps(self.permit()) + "\n", encoding="utf-8"
            )

            waited, observation = wait_for_turn_permit(
                gate_dir, 2, "session-affinity"
            )

            self.assertGreaterEqual(waited, 0.0)
            self.assertEqual(observation["turn"], 2)
            self.assertEqual(observation["session_id"], "session-affinity")
            self.assertTrue((gate_dir / "waiting-turn-02.json").is_file())

    def test_permit_fails_closed_at_provider_cap(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            gate_dir = Path(temp)
            (gate_dir / "permit-turn-01.json").write_text(
                json.dumps(self.permit(codex=23.0)) + "\n", encoding="utf-8"
            )

            with self.assertRaisesRegex(RuntimeError, "denied by quota gate"):
                wait_for_turn_permit(gate_dir, 1, None)


class SessionStateTests(unittest.TestCase):
    def setUp(self) -> None:
        self.database = sqlite3.connect(":memory:")
        self.database.executescript(
            """
            CREATE TABLE session (
                id TEXT PRIMARY KEY,
                cwd TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                parent_session_id TEXT,
                agent_active INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE message (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                seq INTEGER NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                model TEXT,
                visibility TEXT NOT NULL DEFAULT 'llm'
            );
            CREATE TABLE tool_call (
                message_id TEXT NOT NULL
            );
            INSERT INTO session (id, cwd, created_at) VALUES ('session-1', '/workspace', 10);
            """
        )

    def tearDown(self) -> None:
        self.database.close()

    def state(self, baseline_seq: int, *, expected_active: int = 0) -> bool:
        session_id, active, complete = session_state(
            self.database,
            "/workspace",
            0,
            "session-1",
            baseline_seq,
        )
        self.assertEqual(session_id, "session-1")
        self.assertEqual(active, expected_active)
        return complete

    def insert(
        self,
        seq: int,
        role: str,
        content: str,
        *,
        model: str | None = None,
        visibility: str = "llm",
    ) -> None:
        self.database.execute(
            "INSERT INTO message "
            "(id, session_id, seq, role, content, model, visibility) "
            "VALUES (?, ?, ?, ?, ?, ?, ?)",
            (f"message-{seq}", "session-1", seq, role, content, model, visibility),
        )

    def test_delayed_previous_turn_marker_does_not_complete_new_prompt(self) -> None:
        self.insert(1, "user", "new prompt")
        self.insert(2, "system", "memory")

        self.assertFalse(self.state(0))

    def test_current_final_response_completes_when_agent_is_idle(self) -> None:
        self.insert(
            1,
            "assistant",
            "finished",
            model="codex-oauth::gpt-5.6-luna",
            visibility="llm_only",
        )

        self.assertTrue(self.state(0))

    def test_empty_tool_call_assistant_does_not_complete_turn(self) -> None:
        self.insert(
            1,
            "assistant",
            "",
            model="claude-cli::opus[1m]",
            visibility="llm",
        )
        self.insert(2, "tool", "tool result")
        self.insert(3, "system", "suggest")

        self.assertFalse(self.state(0))

    def test_detached_marker_order_does_not_delay_idle_final_response(self) -> None:
        self.insert(1, "system", "memory")
        self.insert(
            2,
            "assistant",
            "finished",
            model="claude-cli::opus[1m]",
            visibility="llm_only",
        )

        self.assertTrue(self.state(0))

    def test_nonempty_tool_call_response_does_not_complete_turn(self) -> None:
        self.insert(
            1,
            "assistant",
            "intermediate text",
            model="codex-oauth::gpt-5.6-sol",
        )
        self.database.execute(
            "INSERT INTO tool_call (message_id) VALUES ('message-1')"
        )

        self.assertFalse(self.state(0))

    def test_agent_returning_idle_is_not_completion_without_response(self) -> None:
        self.database.execute(
            "UPDATE session SET agent_active = 1 WHERE id = 'session-1'"
        )
        self.assertFalse(self.state(0, expected_active=1))
        self.database.execute(
            "UPDATE session SET agent_active = 0 WHERE id = 'session-1'"
        )
        self.insert(1, "system", "recap")

        self.assertFalse(self.state(0))

    def test_prompt_persistence_is_bound_to_exact_current_user_turn(self) -> None:
        self.insert(1, "user", "older prompt")
        self.insert(2, "user", "current prompt")

        self.assertTrue(
            prompt_persisted(self.database, "session-1", 1, "current prompt")
        )
        self.assertFalse(
            prompt_persisted(self.database, "session-1", 2, "current prompt")
        )
        self.assertFalse(
            prompt_persisted(self.database, "session-1", 1, "different prompt")
        )


class TimeoutTests(unittest.TestCase):
    def test_turn_and_total_timeouts_are_independent(self) -> None:
        self.assertIsNone(timeout_kind(1_400, 1_400, 9_000, 1_500))
        self.assertEqual(timeout_kind(1_501, 1_501, 9_000, 1_500), "turn")
        self.assertEqual(timeout_kind(9_001, 200, 9_000, 1_500), "total")

    def test_surface_state_distinguishes_composer_from_workflow_modal(self) -> None:
        self.assertEqual(surface_state("Message…  Commands"), (True, False))
        self.assertEqual(
            surface_state("⛓ workflow · 1 agent · Esc background (^O reopens)"),
            (False, True),
        )
        self.assertEqual(
            surface_state(
                "stale Message… Commands ⛓ workflow · Esc background (^O reopens)"
            ),
            (False, True),
            "a modal must win over stale composer text retained from the previous frame",
        )

    def test_terminal_turn_failure_extracts_visible_error(self) -> None:
        self.assertEqual(
            terminal_turn_failure(
                "status chrome\nTurn failed: provider request failed: retry later\ncomposer"
            ),
            "provider request failed: retry later",
        )
        self.assertIsNone(terminal_turn_failure("turn finished normally"))


class HarnessIntegrationTests(unittest.TestCase):
    def test_workflow_modal_is_backgrounded_before_the_next_prompt(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            database_path = root / "forge.db"
            with sqlite3.connect(database_path) as database:
                database.executescript(
                    """
                    CREATE TABLE session (
                        id TEXT PRIMARY KEY,
                        cwd TEXT NOT NULL,
                        created_at INTEGER NOT NULL,
                        parent_session_id TEXT,
                        agent_active INTEGER NOT NULL DEFAULT 0
                    );
                    CREATE TABLE message (
                        id TEXT PRIMARY KEY,
                        session_id TEXT NOT NULL,
                        seq INTEGER NOT NULL,
                        role TEXT NOT NULL,
                        content TEXT NOT NULL,
                        model TEXT,
                        visibility TEXT NOT NULL DEFAULT 'llm'
                    );
                    CREATE TABLE tool_call (
                        message_id TEXT NOT NULL
                    );
                    """
                )
            prompts = root / "prompts.json"
            prompts.write_text(
                json.dumps(["first prompt", "second prompt"]), encoding="utf-8"
            )
            gate_dir = root / "quota-gates"
            gate_dir.mkdir()
            permit = {
                "allow": True,
                "observed_at": "2026-07-28T12:00:00Z",
                "codex_weekly_percent": 12.0,
                "codex_hard_stop_percent": 22.0,
                "claude_weekly_percent": 29.0,
                "claude_hard_stop_percent": 32.0,
            }
            (gate_dir / "permit-turn-02.json").write_text(
                json.dumps(permit) + "\n", encoding="utf-8"
            )
            def write_delayed_permit() -> None:
                waiting = gate_dir / "waiting-turn-01.json"
                deadline = time.monotonic() + 5.0
                while not waiting.is_file():
                    if time.monotonic() >= deadline:
                        return
                    time.sleep(0.01)
                time.sleep(0.3)
                (gate_dir / "permit-turn-01.json").write_text(
                    json.dumps(permit) + "\n", encoding="utf-8"
                )

            delayed_permit = threading.Thread(target=write_delayed_permit)
            child = root / "fake_tui.py"
            child.write_text(
                r"""
import os
import sqlite3
import sys
import time
import tty

database_path, cwd = sys.argv[1:]
session_id = "session-integration"
database = sqlite3.connect(database_path, timeout=5)
database.execute(
    "INSERT INTO session (id, cwd, created_at) VALUES (?, ?, ?)",
    (session_id, cwd, int(time.time())),
)
database.commit()
tty.setraw(sys.stdin.fileno())

def read_prompt():
    data = b""
    marker = b"\x1b[201~\r"
    while marker not in data:
        data += os.read(sys.stdin.fileno(), 4096)
    start = data.rfind(b"\x1b[200~")
    return data[start + len(b"\x1b[200~") : data.index(marker, start)].decode()

def finish_turn(sequence, prompt):
    database.executemany(
        "INSERT INTO message "
        "(session_id, seq, role, content, model, visibility) VALUES (?, ?, ?, ?, ?, ?)",
        [
            (session_id, sequence, "user", prompt, None, "llm"),
            (session_id, sequence + 1, "assistant", "done", "fake::model", "llm_only"),
            (session_id, sequence + 2, "system", "recap", None, "llm"),
        ],
    )
    database.commit()

os.write(sys.stdout.fileno(), b"\x1b[6n")
time.sleep(0.05)
os.write(sys.stdout.fileno(), b"Message... Commands\n")
finish_turn(1, read_prompt())
os.write(
    sys.stdout.fileno(),
    b"workflow - finished - Esc background (^O reopens)\n",
)
while b"\x1b" not in os.read(sys.stdin.fileno(), 4096):
    pass
os.write(sys.stdout.fileno(), b"Message... Commands\n")
finish_turn(4, read_prompt())
os.write(sys.stdout.fileno(), b"Message... Commands\n")
while b"\x1b" not in os.read(sys.stdin.fileno(), 4096):
    pass
database.close()
""".lstrip(),
                encoding="utf-8",
            )
            harness = Path(__file__).with_name("pty_chat_harness.py")
            delayed_permit.start()
            try:
                completed = subprocess.run(
                    [
                        sys.executable,
                        str(harness),
                        "--cwd",
                        str(root),
                        "--prompt-sequence-file",
                        str(prompts),
                        "--log-prefix",
                        str(root / "live"),
                        "--db",
                        str(database_path),
                        "--timeout",
                        "8",
                        "--turn-timeout",
                        "3",
                        "--settle",
                        "0.05",
                        "--turn-gate-dir",
                        str(gate_dir),
                        "--",
                        sys.executable,
                        str(child),
                        str(database_path),
                        str(root),
                    ],
                    check=False,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.STDOUT,
                    text=True,
                    timeout=12,
                )
            finally:
                delayed_permit.join()

            self.assertEqual(completed.returncode, 0, completed.stdout)
            summary = json.loads(completed.stdout.splitlines()[-1])
            self.assertEqual(summary["turns_completed"], 2)
            self.assertFalse(summary["prompt_dispatch_failed"])
            self.assertGreaterEqual(summary["quota_gate_wait_s"], 0.2)
            self.assertEqual(len(summary["quota_gates"]), 2)
            timeline = (root / "live.timeline.tsv").read_text(encoding="utf-8")
            self.assertIn("MODAL_DISMISS turn=1 kind=workflow", timeline)
            with sqlite3.connect(database_path) as database:
                user_prompts = [
                    row[0]
                    for row in database.execute(
                        "SELECT content FROM message WHERE role = 'user' ORDER BY seq"
                    )
                ]
            self.assertEqual(user_prompts, ["first prompt", "second prompt"])


if __name__ == "__main__":
    unittest.main()
