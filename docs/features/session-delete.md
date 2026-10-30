# Deleting a session for good

Archiving hides a session; deleting removes it. Every surface can delete: the apps (long-press a
session, Delete permanently), the CLI, the chat TUI and the project board. They all run the same
checks, in `crates/forge-cli/src/session_delete.rs`, and the same store call,
`Store::delete_session_report` (`crates/forge-store/src/session_delete_store.rs`).

## Where

| Surface | How |
|---|---|
| CLI | `forge sessions --delete <id-or-prefix> [--yes] [--compact-db]` |
| Chat TUI, this session | `/delete`: lists what goes, then you type the session's short id |
| Chat TUI, any session | sessions picker (`/sessions`, `/resume`, `forge chat --resume`): Ctrl+W (Alt+Backspace) on a row |
| Board | Del on a card, then confirm; the daemon's refusal appears as the toast |
| Apps | session menu, Delete permanently |

The CLI is a flag, not a `delete` subcommand, because `forge sessions` already takes `--unarchive`
and the list filters as flags; `--delete` sits beside them and conflicts with them. It prints the
title, turns, subagent count and worktree, then asks for the first 8 characters of the id
(`--yes` skips the prompt; without a terminal it refuses rather than guess).

With a daemon running, the CLI, board and chat call `DELETE /api/sessions/:id` so the daemon's
registry-aware checks apply. Without one, the same checks run in-process.

## When it refuses

Every refusal says why and what to do. Nothing is deleted.

| Reason | What to do |
|---|---|
| Open in a `forge chat` terminal (named by host) | run `/delete` in that terminal, or close the chat and delete it here |
| Running in the daemon | stop or archive it first |
| Still owns a worktree that exists | resume it and merge or discard the worktree first |
| Frozen by an Anywhere handoff | finish or cancel the handoff |

`/delete` on the chat's own session stops the running turn and drops queued prompts, moves the chat
to a fresh session (which releases this terminal's claim), then deletes the old one through the
same checks. If a check refuses at that point the old session is intact and `/resume <id>` brings
it back.

## What is removed

- The session and its whole subagent tree: messages, tool calls, routing decisions, usage,
  checkpoints, tasks, compaction summaries, live events, workflow runs, heartbeats, imported-session
  and Claude-session links (foreign-key cascades).
- Rows with no foreign key: live-activity tokens, mesh outcomes, harness refinements, messages
  queued to it, and dispatch plans it coordinated.
- Its sync copies. If it was synced, a tombstone is written so other devices drop it; the journal's
  earlier content revisions and queued upload envelopes are purged.
- Files: the rewind snapshots (`<state>/checkpoints/<session>/`) and the evidence ledger
  (`<state>/evidence/<session>/`) under the session's workspace and worktree state folders, plus
  the legacy `<workspace>/.forge/checkpoints/<session>/`.

## What stays

- Queue history, schedule run history and dispatch items keep their rows with the session id
  cleared.
- Saved memories distilled from the session. They are your notes; `/memory` removes them. The
  confirmation counts them.
- A worktree on disk (the delete refuses while it exists).
- Whole-database copies: the pre-migration copy and anything `forge backup` wrote still contain
  the session.
- What `claude` or `codex` wrote for themselves under their own ids (`~/.claude/projects`, codex
  sessions). A bridge session keeps no Forge-side per-session files: the MCP event sink is a
  temp file named by process, not session, and the bridge home is shared. Forge does not record
  the CLI's own session id on disk, so it cannot find those transcripts to remove them.
- Unlinked files are not shredded. Only the database is scrubbed.

## Scrubbing the database

SQLite does not erase deleted rows: their pages go to a free list with the text intact, and the
write-ahead log keeps older page images. So the delete runs with `PRAGMA secure_delete = ON`
(freed content is overwritten with zeros, and the setting is restored afterwards, so ordinary
writes pay nothing) and finishes with `PRAGMA wal_checkpoint(TRUNCATE)` so no earlier image of
the content is left in the `-wal` file. A test reads the `.db` and `-wal` bytes after a delete to
check the text is gone. If another process holds the log open the truncate cannot finish; the
delete reports that and the next checkpoint completes it.

`secure_delete` zeroes the cells it frees, but SQLite's b-tree rebalancing can rewrite a page
and leave a fragment of the old cell in its unused space (reproduced on a small database), so the
default scrub is best effort. `VACUUM` rebuilds every page and is the exact answer; it also
shrinks the file. `--compact-db` runs it after the delete. It rewrites the
whole database and needs exclusive access, so it is CLI-only, explicit, and refused while the
daemon or any `forge chat` is running. Forge never vacuums a store another process holds.
