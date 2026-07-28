#!/usr/bin/env python3
"""Drive a real Forge chat TUI through a pseudoterminal and record redraw timing."""

from __future__ import annotations

import argparse
import errno
import fcntl
import json
import os
import pty
import re
import select
import signal
import sqlite3
import struct
import sys
import termios
import time
from pathlib import Path


CSI_RE = re.compile(rb"\x1b\[[0-?]*[ -/]*[@-~]")
OSC_RE = re.compile(rb"\x1b\][^\x07]*(?:\x07|\x1b\\)")
INTERESTING = re.compile(
    r"(?i)(preparing turn|provider request|receiving provider stream|streaming response|"
    r"running tool|processing tool result|auxiliary mode|finalizing turn|subagent|"
    r"coordinating work|no events for|reported progress|failover|retrying provider|"
    r"warning|error|interrupt|stopped responding|turn finished|turn completed|autofix|recap)"
)


def printable_excerpt(chunk: bytes) -> str:
    clean = OSC_RE.sub(b"", CSI_RE.sub(b"", chunk))
    clean = clean.replace(b"\r", b"\n")
    text = clean.decode("utf-8", "replace")
    lines = [" ".join(line.split()) for line in text.splitlines()]
    interesting = [line for line in lines if line and INTERESTING.search(line)]
    return " | ".join(interesting[-3:])[:900]


def write_all(fd: int, data: bytes) -> None:
    view = memoryview(data)
    while view:
        try:
            written = os.write(fd, view)
            view = view[written:]
        except BlockingIOError:
            select.select([], [fd], [], 0.25)


def timeout_kind(
    elapsed: float, turn_elapsed: float, total_limit: float, turn_limit: float | None
) -> str | None:
    if turn_limit is not None and turn_elapsed >= turn_limit:
        return "turn"
    if elapsed >= total_limit:
        return "total"
    return None


def surface_state(text: str) -> tuple[bool, bool]:
    """Return (composer_visible, workflow_modal_visible) from recent terminal text."""
    lowered = text.lower()
    workflow = "workflow" in lowered and "esc background" in lowered
    composer = (
        not workflow
        and ("message…" in lowered or "message..." in lowered)
        and "commands" in lowered
    )
    return composer, workflow


def prompt_persisted(
    connection: sqlite3.Connection,
    session_id: str,
    baseline_seq: int,
    prompt: str,
) -> bool:
    return (
        connection.execute(
            "SELECT 1 FROM message "
            "WHERE session_id = ? AND seq > ? AND role = 'user' AND content = ? LIMIT 1",
            (session_id, baseline_seq, prompt),
        ).fetchone()
        is not None
    )


def session_state(
    connection: sqlite3.Connection,
    cwd: str,
    started_at: int,
    requested_session_id: str | None,
    baseline_seq: int,
) -> tuple[str | None, int | None, bool]:
    if requested_session_id:
        row = connection.execute(
            "SELECT id, agent_active FROM session WHERE id = ? AND cwd = ?",
            (requested_session_id, cwd),
        ).fetchone()
    else:
        row = connection.execute(
            "SELECT id, agent_active FROM session "
            "WHERE cwd = ? AND created_at >= ? AND parent_session_id IS NULL "
            "ORDER BY created_at DESC LIMIT 1",
            (cwd, started_at),
        ).fetchone()
    if row is None:
        return None, None, False
    session_id = str(row[0])
    # Interactive chat does not currently toggle session.agent_active. Recap/suggestion/memory
    # records are post-turn signals, but a slow background record from the previous turn can land
    # after the next user prompt. Require both the current turn's persisted final assistant response
    # and a later post-turn record so delayed background work cannot advance a prompt sequence.
    post_turn = (
        connection.execute(
            "SELECT 1 FROM message AS response "
            "WHERE response.session_id = ? AND response.seq > ? "
            "AND response.role = 'assistant' AND response.visibility = 'llm_only' "
            "AND response.model IS NOT NULL AND length(trim(response.content)) > 0 "
            "AND EXISTS ("
            "  SELECT 1 FROM message AS marker "
            "  WHERE marker.session_id = response.session_id "
            "  AND marker.seq > response.seq AND marker.role = 'system' "
            "  AND lower(trim(marker.content)) IN ('recap', 'suggest', 'memory')"
            ") LIMIT 1",
            (session_id, baseline_seq),
        ).fetchone()
        is not None
    )
    return session_id, int(row[1]), post_turn


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--cwd", required=True)
    prompts = parser.add_mutually_exclusive_group(required=True)
    prompts.add_argument("--prompt-file")
    prompts.add_argument(
        "--prompt-sequence-file",
        help="JSON array of prompts to send as consecutive turns in one live TUI session",
    )
    parser.add_argument(
        "--prompt-start",
        type=int,
        default=0,
        help="zero-based first prompt to send from --prompt-sequence-file",
    )
    parser.add_argument("--log-prefix", required=True)
    data_home = Path(
        os.environ.get("XDG_DATA_HOME", Path.home() / ".local" / "share")
    )
    parser.add_argument("--db", default=str(data_home / "forge" / "forge.db"))
    parser.add_argument("--timeout", type=float, default=900.0)
    parser.add_argument(
        "--turn-timeout",
        type=float,
        help="maximum wall time for any one prompt; independent of the whole-run timeout",
    )
    parser.add_argument("--settle", type=float, default=2.0)
    parser.add_argument("--session-id")
    parser.add_argument(
        "--turn-offset",
        type=int,
        default=0,
        help="number added to reported turn indices when resuming part of a prompt sequence",
    )
    parser.add_argument("--interrupt-after", type=float)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()

    command = args.command
    if command and command[0] == "--":
        command = command[1:]
    if not command:
        parser.error("a command is required after --")

    cwd = str(Path(args.cwd).resolve())
    if args.prompt_sequence_file:
        loaded = json.loads(Path(args.prompt_sequence_file).read_text(encoding="utf-8"))
        if (
            not isinstance(loaded, list)
            or not loaded
            or not all(isinstance(prompt, str) and prompt.strip() for prompt in loaded)
        ):
            parser.error(
                "--prompt-sequence-file must contain a non-empty JSON string array"
            )
        if args.prompt_start < 0 or args.prompt_start >= len(loaded):
            parser.error("--prompt-start is outside the prompt sequence")
        prompt_texts = loaded[args.prompt_start :]
        prompt_bodies = [prompt.encode() for prompt in prompt_texts]
    else:
        if args.prompt_start:
            parser.error("--prompt-start requires --prompt-sequence-file")
        prompt_bodies = [Path(args.prompt_file).read_bytes().rstrip(b"\n")]
        prompt_texts = [prompt_bodies[0].decode("utf-8")]
    # Crossterm exposes bracketed content as one Paste event. This is essential for multiline
    # prompts: raw embedded LF bytes are editor events, not a paste, and can leave the final Enter
    # sitting inside the editor instead of submitting the composed message.
    encoded_prompts = [
        b"\x1b[200~" + prompt_body + b"\x1b[201~\r" for prompt_body in prompt_bodies
    ]
    prefix = Path(args.log_prefix)
    raw_path = prefix.with_suffix(".raw")
    timeline_path = prefix.with_suffix(".timeline.tsv")

    started_wall = int(time.time()) - 2
    started = time.monotonic()
    pid, master = pty.fork()
    if pid == 0:
        os.chdir(cwd)
        env = os.environ.copy()
        env.setdefault("TERM", "xterm-256color")
        env.setdefault("COLORTERM", "truecolor")
        os.execvpe(command[0], command, env)

    fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", 44, 150, 0, 0))
    flags = fcntl.fcntl(master, fcntl.F_GETFL)
    fcntl.fcntl(master, fcntl.F_SETFL, flags | os.O_NONBLOCK)

    database = sqlite3.connect(f"file:{args.db}?mode=ro", uri=True, timeout=5)
    session_id: str | None = args.session_id
    baseline_seq = -1
    # A resumed session already ends in a recap/suggestion/memory marker. Establish its baseline
    # before the first state poll so that old completion metadata cannot terminate the harness
    # before the TUI is ready and the new prompt has been submitted.
    if session_id is not None:
        row = database.execute(
            "SELECT coalesce(max(seq), -1) FROM message WHERE session_id = ?",
            (session_id,),
        ).fetchone()
        baseline_seq = int(row[0]) if row else -1
    active: int | None = None
    any_active_seen = False
    any_completion_marker_seen = False
    finished_at: float | None = None
    turn_index = 0
    turn_summaries: list[dict[str, object]] = []
    prompt_sent = False
    prompt_sent_at: float | None = None
    current_prompt_persisted = False
    prompt_dispatch_failed = False
    interrupt_sent = False
    dsr_seen = False
    ui_ready_at: float | None = None
    escape_sent = False
    timed_out = False
    timeout_reason: str | None = None
    timeout_triggered_at: float | None = None
    read_events = 0
    total_bytes = 0
    last_read: float | None = None
    max_active_gap = 0.0
    turn_max_active_gap = 0.0
    last_state_poll = 0.0
    dsr_tail = b""
    state_tail = ""
    workflow_visible = False
    waiting_for_composer_after_modal = False
    composer_visible = False
    child_status: int | None = None

    raw_path.parent.mkdir(parents=True, exist_ok=True)
    with (
        raw_path.open("wb") as raw,
        timeline_path.open("w", encoding="utf-8") as timeline,
    ):
        timeline.write("elapsed_s\tbytes\tgap_s\tagent_active\texcerpt\n")
        while True:
            now = time.monotonic()
            elapsed = now - started

            # Ratatui asks the terminal for its cursor position during startup. Do not queue the
            # prompt before answering that query: a slow provider/catalog startup can otherwise
            # make the TUI consume the prompt itself as the DSR response and abort initialization.
            if not prompt_sent and ui_ready_at is not None and now >= ui_ready_at:
                if session_id is None:
                    found_id, _, _ = session_state(
                        database, cwd, started_wall, None, baseline_seq
                    )
                    session_id = found_id
                if session_id is not None:
                    row = database.execute(
                        "SELECT coalesce(max(seq), -1) FROM message WHERE session_id = ?",
                        (session_id,),
                    ).fetchone()
                    baseline_seq = int(row[0]) if row else -1
                write_all(master, encoded_prompts[turn_index])
                prompt_sent = True
                prompt_sent_at = now
                current_prompt_persisted = False
                timeline.write(
                    f"{elapsed:.3f}\t0\t0.000\t{active}\t"
                    f"PROMPT_SENT turn={args.turn_offset + turn_index + 1}/"
                    f"{args.turn_offset + len(encoded_prompts)}\n"
                )
                timeline.flush()

            if now - last_state_poll >= 0.5:
                last_state_poll = now
                try:
                    found_id, found_active, post_turn = session_state(
                        database,
                        cwd,
                        started_wall,
                        args.session_id,
                        baseline_seq,
                    )
                    if found_id is not None:
                        session_id, active = found_id, found_active
                        if prompt_sent and not current_prompt_persisted:
                            current_prompt_persisted = prompt_persisted(
                                database,
                                session_id,
                                baseline_seq,
                                prompt_texts[turn_index],
                            )
                        if active == 1:
                            any_active_seen = True
                        if post_turn:
                            any_completion_marker_seen = True
                        # Activity is telemetry, not a completion proof: startup failures and
                        # interrupted turns may also return to idle. Only a persisted non-empty
                        # final response followed by post-turn bookkeeping may advance the suite.
                        if post_turn and finished_at is None:
                            finished_at = now
                            turn_summaries.append(
                                {
                                    "turn": args.turn_offset + turn_index + 1,
                                    "elapsed_s": round(
                                        now - (prompt_sent_at or started), 3
                                    ),
                                    "completed": True,
                                    "interrupted": False,
                                    "max_output_gap_s": round(turn_max_active_gap, 3),
                                }
                            )
                            timeline.write(
                                f"{elapsed:.3f}\t0\t0.000\t0\t"
                                f"TURN_FINISHED turn={args.turn_offset + turn_index + 1} "
                                f"{session_id}\n"
                            )
                            timeline.flush()
                except sqlite3.OperationalError as exc:
                    timeline.write(
                        f"{elapsed:.3f}\t0\t0.000\t{active}\tDB_BUSY {exc}\n"
                    )

            if (
                args.interrupt_after is not None
                and prompt_sent_at is not None
                and not interrupt_sent
                and finished_at is None
                and now - prompt_sent_at >= args.interrupt_after
            ):
                write_all(master, b"\x1b")
                interrupt_sent = True
                timeline.write(f"{elapsed:.3f}\t0\t0.000\t{active}\tINTERRUPT_SENT\n")
                timeline.flush()

            readable, _, _ = select.select([master], [], [], 0.1)
            if readable:
                try:
                    chunk = os.read(master, 65536)
                except OSError as exc:
                    if exc.errno == errno.EIO:
                        chunk = b""
                    else:
                        raise
                except BlockingIOError:
                    chunk = b""
                if chunk:
                    now = time.monotonic()
                    gap = 0.0 if last_read is None else now - last_read
                    if prompt_sent and finished_at is None:
                        max_active_gap = max(max_active_gap, gap)
                        turn_max_active_gap = max(turn_max_active_gap, gap)
                    last_read = now
                    read_events += 1
                    total_bytes += len(chunk)
                    raw.write(chunk)
                    raw.flush()
                    combined = dsr_tail + chunk
                    queries = combined.count(b"\x1b[6n")
                    if queries:
                        write_all(master, b"\x1b[1;1R" * queries)
                        dsr_seen = True
                        if ui_ready_at is None:
                            ui_ready_at = now + 0.35
                    # Keep only a possible partial DSR prefix, never a complete query (which would
                    # be counted and answered again when the next chunk arrives).
                    dsr_tail = combined[-3:]
                    clean_state = OSC_RE.sub(b"", CSI_RE.sub(b"", chunk)).decode(
                        "utf-8", "replace"
                    )
                    # One 44×150 full-screen frame is roughly 6,600 characters. Keep enough recent
                    # terminal text to correlate the workflow header with its `Esc background`
                    # footer even when the PTY splits that frame across several reads.
                    state_tail = (state_tail + clean_state.lower())[-16_000:]
                    saw_composer, saw_workflow = surface_state(state_tail)
                    if saw_workflow:
                        workflow_visible = True
                        composer_visible = False
                    if saw_composer:
                        composer_visible = True
                        workflow_visible = False
                    # Newer Crossterm/Ratatui startup paths do not always emit a cursor-position
                    # query. The rendered composer is an equally strong readiness signal and avoids
                    # deadlocking resumed-session tests while waiting for a DSR that will never come.
                    if (
                        ui_ready_at is None
                        and composer_visible
                    ):
                        ui_ready_at = now + 0.35
                    if (
                        interrupt_sent
                        and finished_at is None
                        and "interrupted" in state_tail
                        and "stopped responding" in state_tail
                    ):
                        finished_at = now
                        turn_summaries.append(
                            {
                                "turn": args.turn_offset + turn_index + 1,
                                "elapsed_s": round(
                                    now - (prompt_sent_at or started), 3
                                ),
                                "completed": False,
                                "interrupted": True,
                                "max_output_gap_s": round(turn_max_active_gap, 3),
                            }
                        )
                        timeline.write(
                            f"{now - started:.3f}\t0\t0.000\t{active}\t"
                            f"TURN_INTERRUPTED turn={args.turn_offset + turn_index + 1} "
                            f"{session_id}\n"
                        )
                        timeline.flush()
                    excerpt = printable_excerpt(chunk).replace("\t", " ")
                    if excerpt or gap >= 1.0:
                        timeline.write(
                            f"{now - started:.3f}\t{len(chunk)}\t{gap:.3f}\t{active}\t{excerpt}\n"
                        )
                        timeline.flush()

            if (
                finished_at is not None
                and not escape_sent
                and now - finished_at >= args.settle
            ):
                if workflow_visible:
                    write_all(master, b"\x1b")
                    workflow_visible = False
                    waiting_for_composer_after_modal = True
                    composer_visible = False
                    state_tail = ""
                    timeline.write(
                        f"{elapsed:.3f}\t0\t0.000\t{active}\t"
                        f"MODAL_DISMISS turn={args.turn_offset + turn_index + 1} "
                        "kind=workflow\n"
                    )
                    timeline.flush()
                    continue
                if waiting_for_composer_after_modal and not composer_visible:
                    continue
                waiting_for_composer_after_modal = False
                if turn_index + 1 < len(encoded_prompts):
                    turn_index += 1
                    if session_id is not None:
                        row = database.execute(
                            "SELECT coalesce(max(seq), -1) FROM message WHERE session_id = ?",
                            (session_id,),
                        ).fetchone()
                        baseline_seq = int(row[0]) if row else -1
                    finished_at = None
                    prompt_sent_at = now
                    current_prompt_persisted = False
                    turn_max_active_gap = 0.0
                    last_read = now
                    write_all(master, encoded_prompts[turn_index])
                    timeline.write(
                        f"{elapsed:.3f}\t0\t0.000\t{active}\t"
                        f"PROMPT_SENT turn={args.turn_offset + turn_index + 1}/"
                        f"{args.turn_offset + len(encoded_prompts)}\n"
                    )
                    timeline.flush()
                else:
                    write_all(master, b"\x1b")
                    escape_sent = True

            waited_pid, status = os.waitpid(pid, os.WNOHANG)
            if waited_pid == pid:
                child_status = status
                break

            turn_elapsed = (
                now - prompt_sent_at
                if prompt_sent_at is not None and finished_at is None
                else 0.0
            )
            dispatch_timed_out = (
                prompt_sent_at is not None
                and finished_at is None
                and not current_prompt_persisted
                and now - prompt_sent_at >= 15.0
            )
            timeout = (
                "prompt-dispatch"
                if dispatch_timed_out
                else timeout_kind(elapsed, turn_elapsed, args.timeout, args.turn_timeout)
            )
            if timeout is not None:
                timed_out = True
                prompt_dispatch_failed = prompt_dispatch_failed or dispatch_timed_out
                if timeout_triggered_at is None:
                    timeout_triggered_at = now
                    timeout_reason = timeout
                    timeline.write(
                        f"{elapsed:.3f}\t0\t0.000\t{active}\t"
                        f"TIMEOUT kind={timeout_reason} "
                        f"turn={args.turn_offset + turn_index + 1} "
                        f"turn_elapsed={turn_elapsed:.3f}\n"
                    )
                    timeline.flush()
                if not escape_sent:
                    write_all(master, b"\x1b")
                    escape_sent = True
                if now - timeout_triggered_at >= 2.0:
                    os.kill(pid, signal.SIGTERM)

    database.close()
    os.close(master)

    if child_status is None:
        _, child_status = os.waitpid(pid, 0)
    if os.WIFEXITED(child_status):
        exit_code = os.WEXITSTATUS(child_status)
    elif os.WIFSIGNALED(child_status):
        exit_code = 128 + os.WTERMSIG(child_status)
    else:
        exit_code = 1

    summary = {
        "session_id": session_id,
        "elapsed_s": round(time.monotonic() - started, 3),
        "active_seen": any_active_seen,
        "dsr_seen": dsr_seen,
        "completion_marker_seen": any_completion_marker_seen,
        "interrupt_sent": interrupt_sent,
        "max_turn_output_gap_s": round(max_active_gap, 3),
        "turns_expected": args.turn_offset + len(encoded_prompts),
        "prior_turns_completed": args.turn_offset,
        "turns_completed": args.turn_offset
        + sum(bool(turn["completed"]) for turn in turn_summaries),
        "turns": turn_summaries,
        "read_events": read_events,
        "output_bytes": total_bytes,
        "timed_out": timed_out,
        "timeout_kind": timeout_reason,
        "prompt_dispatch_failed": prompt_dispatch_failed,
        "child_exit_code": exit_code,
        "raw_log": str(raw_path),
        "timeline_log": str(timeline_path),
    }
    print(json.dumps(summary, sort_keys=True))
    if timed_out:
        return 124
    return exit_code


if __name__ == "__main__":
    sys.exit(main())
