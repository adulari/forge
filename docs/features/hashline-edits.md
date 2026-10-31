# Hashline edits (`[harness] edit_mode`)

`edit_mode = "replace"` (default) is `edit_file`/`multi_edit` with exact `old` -> `new` text.
`edit_mode = "hashline"` borrows oh-my-pi's anchored edit mode: the model names lines, not text, so
a missed whitespace match cannot fail an edit.

## What the model sees

- `read_file` (single and batched) returns `[path#TAG]` then `N:text` rows. `TAG` is 16 bits of
  FNV-1a over the whole file, trailing spaces/tabs/CR ignored per line. Rows over 1000 characters
  are clipped when shown; the line is still edited whole by number.
- `edit_file` takes `{path, tag, edits}`; each edit is `{start, end?, new}` (replace the lines,
  empty `new` deletes) or `{after, new}` (insert; `0` is the top). Numbers refer to the file as read;
  edits are applied bottom-up so earlier edits never shift later anchors. Overlapping edits are
  refused.
- A stale or missing tag applies nothing and returns the current lines around the first anchor with
  the new tag. A success returns the new tag and the rows around each edit, so the next edit does not
  need a re-read.
- `multi_edit` is not offered; an `old`/`new` call to `edit_file` still works as in replace mode.
- The system prompt gets a short addendum replacing the base prompt's `old`-text guidance.

## Wiring

`ToolRegistry::set_edit_mode` (forge-tools) is called once per session in `Session::build` from
`config.harness.edit_mode`; it swaps the descriptions/schema and scopes a task-local that
`read_file`/`edit_file` read. The tool name stays `edit_file`, so the permission broker, hooks,
checkpoints (`/undo`), diff previews and tool-card rows are unchanged. A family-profile layer only
needs to set `harness.edit_mode` in the config it hands the session. Bridged claude/codex sessions
(`forge mcp-serve`) keep replace mode.

omp keeps `replace` for Kimi, MiMo, MiniMax, DeepSeek, StepFun and similar families
(`packages/coding-agent/src/utils/edit-mode.ts`), so the default stays `replace`.
