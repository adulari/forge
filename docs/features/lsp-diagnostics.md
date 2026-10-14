# Feature: post-edit LSP diagnostics as a delta

> After a write, Forge asks the language server about the edited file and queues an
> `[lsp diagnostics]` hint (see `competitive-wave.md` §1 for the client). This page covers what
> that hint contains. Touches `forge-core` (`lsp_delta.rs`, `lsp_hints.rs`, `tool_dispatch.rs`)
> and `forge-config` (`[lsp] diagnostics`).

## Why

A strict type checker reports a file's whole backlog after every write. On the Python bench
fixture (basedpyright), one block per edit ran 4.7k–20k characters, almost all of it
`reportUnknown*`/`reportMissingParameterType` warnings that existed before the edit. The model
treated it as noise, and each block was re-sent with the transcript on every later request.
`normalize_system_context` deduplicates identical system messages, but a block after a second
edit is never identical to the first (line numbers shift), so the dedup saved nothing here:
measured wire bytes equalled stored bytes in every baseline run.

## What the hint contains (`[lsp] diagnostics = "delta"`, the default)

Forge keeps one snapshot per file per session: the file's text and the diagnostics the model was
last told about.

- **First touch of a file:** every error in the file, plus warnings/info/hints on or within three
  lines of the changed lines, and a total count. The rest of the backlog is only counted.
- **Later edits:** only diagnostics that are new since the snapshot, plus a `N resolved` count.
  Old diagnostics are mapped through a line diff of the old and new text, so a warning that only
  moved because lines were inserted above it is not "new". A diagnostic on a line the edit
  rewrote matches an old one by severity, code and message.
- **Nothing new and nothing resolved:** no hint at all.
- **Errors the edit introduced** (an error in or near the changed lines, or a newly appearing
  error) are always shown, sorted first, and exempt from the soft cap (up to a hard ceiling of
  8,000 characters for an edit that breaks a file's syntax).
- **Bounding:** everything else is capped at 2,000 characters, with a
  `+N more (run the linter or type checker to see all)` tail.
- **Stable order:** introduced errors first, then severity, distance to the edit, line, column,
  code and message, so the same state renders the same bytes (cache-friendly).
- **Compact lines:** the file is named once in the header (workspace-relative), each finding is
  one line (`L12:5 warning (code) message`), multi-line messages are joined, and messages are
  truncated at 300 characters for errors and 160 for the rest.

Snapshots are dropped when the transcript is rewritten (rewind) or compacted, because the model
may no longer have the earlier blocks; the next edit then reports as a first touch.

`full` restores the old behaviour (every diagnostic, every edit). `off` attaches nothing.

```toml
[lsp]
enabled = true
diagnostics = "delta"   # "delta" | "full" | "off"
```

## Paths

- **Direct path:** the hint is queued in `pending_hints` and lands as a system message right after
  the tool result.
- **Codemode (`run_code`):** inner calls go through the same `invoke_tool`, so each inner write's
  value carries the delta hint as `[hint] …` (codemode.md).
- **CLI bridges:** `forge mcp-serve` does not attach LSP diagnostics, so there is nothing to change.

## Safety

Nothing that decides correctness reads this hint. The verification ledger, the phantom-edit
gate, the failure-loop guards and the doom-loop guard all observe the tool result, and auto-lint
runs its own command. The delta only changes what advice the model reads, and errors the edit
introduced still reach it in full.

## Measured

Bench: 6 Python tasks × 3 reps, `inferhub::glm-5.3-flash`, real basedpyright, baseline =
`feat/sessions-goal-worktrees` @ 32e89c47.

| | pass | input tokens | output tokens | calls | wall s | LSP chars on the wire / task | largest block | LSP chars × later requests |
|---|---|---|---|---|---|---|---|---|
| full (before) | 18/18 | 95,230 | 1,004 | 7.9 | 67.5 | 10,875 | 8,524 | 54,347 |
| delta | 18/18 | 76,601 | 900 | 7.5 | 59.2 | 1,716 | 1,141 | 9,135 |

LSP characters on the wire fell 84%, their re-sent weight 83%, and mean input tokens per task
20%, with no change in pass rate.
