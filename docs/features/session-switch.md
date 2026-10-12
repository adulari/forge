# Session switch: one conversation, Claude Code and Forge

`forge switch` carries ONE conversation between Claude Code and Forge, in both directions,
repeatedly, without duplicating or losing history. Leave either tool, run `forge switch` in the
same directory, and the conversation reopens in the other.

```
$ forge switch
Claude Code "fix the parser" (412 messages, active 3m ago) → Forge
```

The line is printed once, then this process is replaced by `forge chat --resume <id>` (or
`claude --resume <id>`, run in the session's directory), so there is nothing to type afterwards.

## Commands

| | |
|---|---|
| `forge switch` | In this directory, take the more recently active of the newest interactive Claude Code session and the newest Forge session and open it in the other tool. |
| `forge switch <session>` | `<session>` is a Forge session id or unique prefix, a Claude Code session id, or a Claude Code `.jsonl` path. Forge ids are resolved first. |
| `--to claude\|forge` | Force the destination. For a linked pair, naming either half works. |
| `--dry-run` | Print the line and what would happen; write nothing. |
| `--no-launch` | Sync, print the command that would continue the conversation, and stop. |
| `--force` | Switch although the session looks busy or open (below). |
| `/claude` (in the Forge TUI) | Leave the TUI exactly as `/quit` does, then switch the session you were in. |
| `forge import claude-session <id\|path>` | The older one-way import. It now records the same link, so the imported session can switch back. |

## Which way, with no argument

Interactive Claude Code sessions of this directory are found under
`$CLAUDE_CONFIG_DIR/projects` (default `~/.claude/projects`), in the directory named after the
working directory with every non-alphanumeric character replaced by `-`. A session counts as
interactive when its first entry says `entrypoint` other than `sdk-*` and it never names an
`mcp__forge__*` tool: Forge's own claude-cli bridge writes `sdk-cli` transcripts into that same
directory and they are never taken for your session.

* Only one tool has a session here: that one is the source.
* Both, and unrelated: the more recently active is the source.
* Both, and linked to each other: the side with turns the other lacks is the source; if both (or
  neither) changed, the newer one; with no difference at all, the tool the last switch opened (you
  are presumably still there), so a bare `forge switch` toggles.

## What is carried

**Claude Code to Forge.** Unlinked, this is `forge import claude-session` (the active chain across
compaction boundaries, the latest compact summary plus its live tail as the model context, older
turns kept as history), and the link is recorded at the end of the file. Linked, only the bytes
after `cc_synced_bytes` are read and converted with the importer's own row conversion (merged split
assistant messages, tool results, placeholders for missing results), then appended to the same
Forge session in one transaction. A new compaction boundary marks the existing rows compacted and
stores the summary exactly like `/compact`. If the new entries do not descend from the last mirrored
one (Claude Code rewound or branched), or a compaction kept verbatim turns from before the range
read, the transcript is re-imported as a NEW Forge session and the link moves to it; one line says so.

**Forge to Claude Code.** Unlinked: a new session file (a temp file, synced, then renamed) holding
what Forge's model would see next: the compaction summary as a `compact_boundary` plus summary
entry, then the live rows, plus a `custom-title` entry. Linked: only rows after `forge_synced_seq`
are appended, in one write followed by an fsync, chained from the file's real current leaf. A Forge
compaction since the last switch appends a boundary and summary and re-emits the live rows after it.

Row mapping, Forge to Claude Code:

* user rows are user entries; assistant rows are one assistant entry with an optional text block and
  the `tool_use` blocks; all of a turn's results go in ONE user entry with one `tool_result` per call;
* `shell`, `read_file`, `edit_file`, `write_file`, `glob`, `search`/`grep`, `web_fetch`,
  `web_search` become `Bash`, `Read`, `Edit`, `Write`, `Glob`, `Grep`, `WebFetch`, `WebSearch` with
  Claude Code's argument names (`path` to `file_path`, `old`/`new` to `old_string`/`new_string`,
  `timeout_secs` to `timeout` in ms); calls already spelled Claude's way are kept; any other tool
  becomes `mcp__forge__<name>`;
* every call gets exactly one result (`[no result recorded]` for an orphan); a result with no call
  is dropped; tool ids are made `^[A-Za-z0-9_-]+$` deterministically and unique per turn (ids that
  are already well-formed and 16+ characters, such as Claude's `toolu_...`, are kept);
* system rows, nudges, display-only (`ui`) copies and reasoning are skipped; two same-role entries in
  a row are merged;
* `version` is the installed `claude --version`, `entrypoint` is `cli`, `gitBranch` is read from
  `.git/HEAD`.

Claude Code to Forge is the importer's mapping unchanged (tool names stay as Claude spelled them).

## Mode and model

* Claude Code to Forge: the transcript's last `permission-mode`: `bypassPermissions` becomes
  `--mode bypass`, `acceptEdits` `accept-edits`, `plan` `plan`; `default`/`auto` leave Forge's own.
  The model pin is the importer's (`claude-cli::<model>` from the last assistant message).
* Forge to Claude Code: a `claude-cli::<model>` pin becomes `--model <model>`; any other pin prints a
  one-line note and Claude Code uses its default. Bypass becomes `--dangerously-skip-permissions`,
  accept-edits and plan `--permission-mode acceptEdits|plan`.

## Link bookkeeping

Table `cc_session_link` (migration 40): `forge_session_id`, `cc_session_id` (each unique, so a
session links to at most one other), `cc_path`, `cc_synced_bytes` (a line boundary), `cc_last_uuid`,
`forge_synced_seq`, `forge_summary` (the compaction summary both sides already reflect) and
`last_target`. Both sides' progress is written together with the rows in one transaction. A half
written last line is left for the next sync.

## Safety

* Refused (clear message, `--force` overrides) when the Forge session is busy or waiting in the
  `forge serve` daemon, is open in another terminal, or the Claude Code session looks open: a running
  interactive `claude` names its id on the command line, or its transcript was written in the last 5
  seconds while one runs in its directory (print-mode `claude -p` runs, which Forge's bridge starts
  for every turn, do not count).
* If both sides gained turns, the hop still happens: the incoming turns follow the other side's
  newest entry, a one-line warning says both sides changed, and nothing is dropped.
* Nothing is written on `--dry-run` or when a hop fails part-way: new Claude Code files appear by
  rename, appends are one complete-lines write, and Forge writes happen in one transaction.

## Limits

* The other side's `/undo` or rewind of rows already mirrored is not mirrored back.
* Forge tool results are text; Claude Code's richer `toolUseResult` metadata is not written.
* Claude Code's `preservedSegment` after a compaction is only followed when it falls in the range
  read; otherwise the session is re-imported as a new Forge session.
* A project path longer than 200 characters needs Claude Code to have created its directory first
  (its name carries a hash this tool does not reproduce).
