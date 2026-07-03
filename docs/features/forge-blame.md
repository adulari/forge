# `forge blame` — AI provenance for every line

> Status: **done** — `forge blame <file>` (whole-file summary), `forge blame <file> --line N`
> (full provenance card for one line), `--json`. No git dependency.

## What it does

Traces lines of AI-written code back to the session/model/turn that wrote them, using
Forge's own recorded tool calls (`write_file`/`edit_file`) instead of git history. Since every
`record_tool_call` already carries the calling message's `model` (and, transitively, its
session and turn `seq`), the store already has everything needed to answer "who (which
model, which session) wrote this line" — `forge blame` is purely the read side of that record.

## Examples

```
$ forge blame src/auth.rs
    1  use std::sync::Arc;                                    (human/unknown)
    2  fn handle_login(req: Request) -> Response {             claude-opus-4-8   a1b2c3d4  3d ago
    3      let token = issue_token(&req.user);                  claude-opus-4-8   a1b2c3d4  3d ago
    4      // hand-tuned retry backoff, don't touch             (human/unknown)

$ forge blame src/auth.rs --line 3
line 3
  text      let token = issue_token(&req.user);
  model     claude-opus-4-8
  session   a1b2c3d4
  turn      seq 12
  when      2026-06-30 14:02:11 (3d ago)
  prompt    add token issuance to the login handler
  assistant Added issue_token and wired it into handle_login.

  → forge replay a1b2c3d4

$ forge blame src/auth.rs --json
[{"line":1,"text":"use std::sync::Arc;","model":null,"session":null,"seq":null,"created_at":null}, …]
```

## How it works

- **Migration `migration_0002`** adds `tool_call.path TEXT` (+ index), populated going forward
  by `record_tool_call` (extracted from `args_json`'s top-level `"path"` key) and backfilled
  best-effort for pre-existing `write_file`/`edit_file` rows on upgrade.
- **`Store::file_edits(filename_suffix)`** returns every recorded `write_file`/`edit_file` call
  whose path matches (as a suffix) the target, joined to the owning session (for `cwd`, to
  resolve a relative `path` the same way the tool itself did) and to the calling assistant
  message's `model` (falling back to that turn's `routing_decision.chosen_model` when the
  message's own `model` is unset).
- **`crates/forge-cli/src/blame.rs`** is pure over `FileEditRow`s + the current file's content —
  `matching_edits` (precise path resolution: canonicalized match, else literal suffix),
  `attribute_lines` (latest-edit-wins: for each current line, the most recent edit whose
  contributed text contained that line verbatim wins; otherwise "(human/unknown)"), and the
  three renderers (`render_blame`, `render_why`, `render_json`) — so all of this is
  unit-tested without a database.
- **`Store::turn_context(session_id, seq)`** backs `--line`'s provenance card: the nearest user
  prompt at or before that turn, plus the assistant message's own content at that turn.

## Limitations

- **90-day store retention** (`RETENTION_HORIZON_SECS`) — lines written by a session old enough
  to have been pruned show as "(human/unknown)", indistinguishable from a genuinely
  human-written line.
- **64KB truncated writes** — a `write_file`/`edit_file` call whose `args_json` was capped at
  insert time (`MAX_RESULT_JSON_BYTES`) is never attributed; a very large generated file can
  therefore blame as partially unknown even though AI wrote all of it.
- **Content-matching heuristic, not a true diff** — attribution matches a current line against
  each edit's contributed text *verbatim* (trimmed). A line that was AI-written but later
  hand-edited even slightly (or reformatted) attributes as unknown rather than "modified".
  Blank lines and lone braces/brackets/parens are always "(human/unknown)" — too ambiguous to
  be worth matching.
- **Migration-boundary rows** — edits recorded before this feature shipped only have `path` data
  if the backfill in `migration_0002` could parse their (possibly already-truncated) `args_json`;
  a row whose args were truncated before reaching the `path` key stays unattributed.
