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
    r"(?i)(working|thinking|routing|tool|writing|reading|running|verifying|retry|failover|complete|warning|error|interrupt|stopped)"
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
    latest = connection.execute(
        "SELECT seq, role, content FROM message WHERE session_id = ? ORDER BY seq DESC LIMIT 1",
        (session_id,),
    ).fetchone()
    # Interactive chat does not currently toggle session.agent_active. These post-turn records are
    # written only after the accepted assistant response and all in-turn tool work have completed.
    post_turn = bool(
        latest
        and int(latest[0]) > baseline_seq
        and latest[1] == "system"
        and str(latest[2]).strip().lower() in {"recap", "suggest", "memory"}
    )
    return session_id, int(row[1]), post_turn


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--cwd", required=True)
    parser.add_argument("--prompt-file", required=True)
    parser.add_argument("--log-prefix", required=True)
    parser.add_argument("--db", default="/home/floris/.local/share/forge/forge.db")
    parser.add_argument("--timeout", type=float, default=900.0)
    parser.add_argument("--settle", type=float, default=2.0)
    parser.add_argument("--session-id")
    parser.add_argument("--interrupt-after", type=float)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()

    command = args.command
    if command and command[0] == "--":
        command = command[1:]
    if not command:
        parser.error("a command is required after --")

    cwd = str(Path(args.cwd).resolve())
    prompt_body = Path(args.prompt_file).read_bytes().rstrip(b"\n")
    # Crossterm exposes bracketed content as one Paste event. This is essential for multiline
    # prompts: raw embedded LF bytes are editor events, not a paste, and can leave the final Enter
    # sitting inside the editor instead of submitting the composed message.
    prompt = b"\x1b[200~" + prompt_body + b"\x1b[201~\r"
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
    active_seen = False
    completion_marker_seen = False
    finished_at: float | None = None
    prompt_sent = False
    prompt_sent_at: float | None = None
    interrupt_sent = False
    dsr_seen = False
    ui_ready_at: float | None = None
    escape_sent = False
    timed_out = False
    read_events = 0
    total_bytes = 0
    last_read: float | None = None
    max_active_gap = 0.0
    last_state_poll = 0.0
    dsr_tail = b""
    state_tail = ""
    child_status: int | None = None

    raw_path.parent.mkdir(parents=True, exist_ok=True)
    with raw_path.open("wb") as raw, timeline_path.open("w", encoding="utf-8") as timeline:
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
                write_all(master, prompt)
                prompt_sent = True
                prompt_sent_at = now
                timeline.write(f"{elapsed:.3f}\t0\t0.000\t{active}\tPROMPT_SENT\n")
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
                        if active == 1:
                            active_seen = True
                        if post_turn:
                            completion_marker_seen = True
                        if (post_turn or (active_seen and active == 0)) and finished_at is None:
                            finished_at = now
                            timeline.write(
                                f"{elapsed:.3f}\t0\t0.000\t0\tTURN_FINISHED {session_id}\n"
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
                timeline.write(
                    f"{elapsed:.3f}\t0\t0.000\t{active}\tINTERRUPT_SENT\n"
                )
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
                    state_tail = (state_tail + clean_state.lower())[-1000:]
                    # Newer Crossterm/Ratatui startup paths do not always emit a cursor-position
                    # query. The rendered composer is an equally strong readiness signal and avoids
                    # deadlocking resumed-session tests while waiting for a DSR that will never come.
                    if ui_ready_at is None and (
                        "message…" in state_tail
                        or "message..." in state_tail
                    ) and "commands" in state_tail:
                        ui_ready_at = now + 0.35
                    if (
                        interrupt_sent
                        and finished_at is None
                        and "interrupted" in state_tail
                        and "stopped responding" in state_tail
                    ):
                        finished_at = now
                        timeline.write(
                            f"{now - started:.3f}\t0\t0.000\t{active}\tTURN_INTERRUPTED {session_id}\n"
                        )
                        timeline.flush()
                    excerpt = printable_excerpt(chunk).replace("\t", " ")
                    if excerpt or gap >= 1.0:
                        timeline.write(
                            f"{now - started:.3f}\t{len(chunk)}\t{gap:.3f}\t{active}\t{excerpt}\n"
                        )
                        timeline.flush()

            if finished_at is not None and not escape_sent and now - finished_at >= args.settle:
                write_all(master, b"\x1b")
                escape_sent = True

            waited_pid, status = os.waitpid(pid, os.WNOHANG)
            if waited_pid == pid:
                child_status = status
                break

            if elapsed >= args.timeout:
                timed_out = True
                if not escape_sent:
                    write_all(master, b"\x1b")
                    escape_sent = True
                if elapsed >= args.timeout + 2.0:
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
        "active_seen": active_seen,
        "dsr_seen": dsr_seen,
        "completion_marker_seen": completion_marker_seen,
        "interrupt_sent": interrupt_sent,
        "max_turn_output_gap_s": round(max_active_gap, 3),
        "read_events": read_events,
        "output_bytes": total_bytes,
        "timed_out": timed_out,
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
