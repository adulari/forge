# Session replay — auditable, reproducible runs

> Status: **done** — `forge replay <id>` (transcript) + `forge replay <a> <b>` (diff),
> read-only over the persisted record, plus `forge replay <id> --rerun` (true model
> re-execution: re-issue the recorded prompts on the current model/mesh and diff vs. the
> original), plus `forge replay <id> --html <path>` / the TUI's `/export [path]` (one
> self-contained HTML file of the transcript).

## Why

The Helm-note vision lists "record prompts + model versions + outputs; replay + diff;
auditable, reproducible AI" as a Wave-4 differentiator. Forge already *records* everything a
turn does — every message (`role`, `content`, `model`, tool calls, timestamp) and its
`usage` row (input/output tokens, `cost_usd`) live in the shared SQLite db. Replay is the
**read side** of that record: reconstruct exactly what happened, and compare two runs.

## What shipped

- **`forge replay <id>`** — turn-by-turn transcript: a header (start time, elapsed seconds,
  prompt/message/model counts, total cost, total tokens) then each message with its role,
  a one-line clip of the content, and `[model · $cost]`; tool calls render as `↳ name(args)`.
  This faithfully shows what the model saw (including Lattice-injected `system` context) and
  what each turn cost.
- **`forge replay <a> <b>`** — summary-level diff aligning two sessions: prompt-count delta,
  cost delta, token totals, and which models were used in one run but not the other. Answers
  "this run cost more / used a different model / took more turns than that one" — the audit
  question when comparing two attempts at the same task.
- Ids accept git-style prefixes (resolved via `Store::matching_session_ids`); ambiguous or
  unknown prefixes error cleanly.

## Design

- **Data:** `Store::load_replay(session_id) -> Vec<ReplayEntry>` — `message LEFT JOIN usage`,
  active rows only, ordered by `seq`. `ReplayEntry` carries `seq/role/content/model/
  created_at/tool_calls` plus optional `input_tokens/output_tokens/cost_usd` (None for
  user/tool messages or pre-usage-tracking turns).
- **Logic:** `crates/forge-cli/src/replay.rs` is pure over `&[ReplayEntry]` — `summarize`,
  `diff`, `render_transcript`, `render_diff` — so it is unit-tested without a database. The
  `Replay` CLI command only resolves ids and prints.

## Shipped (follow-up)

- **`/replay` chat command** — `/replay <id>` shows the transcript inline in the TUI; `/replay
  <a> <b>` shows the diff. Dispatched as `CommandAction::Replay` in forge-tui, handled in
  forge-cli via the existing `replay::render_transcript` / `replay::render_diff` functions.
  Non-mutating, so it can run while a turn is in progress.
- **Per-turn content diff** — `render_turn_diff(id_a, id_b, a, b)` in `replay.rs` aligns
  assistant turns pairwise and shows where content diverged (identical turns marked, additions/
  deletions shown with A:/B: labels). Surfaced by both `forge replay <a> <b>` and `/replay <a>
  <b>` after the summary diff.
- **`Tui::print_text`** — convenience method for pushing plain multi-line strings into the
  terminal scrollback without requiring callers to construct ratatui Line<'static> values.

## Shipped (follow-up 2)

- **`forge replay <id> --json`** — `render_json(id, entries)` in `replay.rs` emits a
  structured JSON object: `{ session_id, summary, turns }`, with each turn carrying seq,
  role, created_at, content, model, token counts, cost_usd, and tool_calls. Suitable for
  external auditing, piping to `jq`, or feeding into analysis scripts.

## Shipped (follow-up 3)

- **`forge replay <id> --html <path>`** — `render_html(id, entries)` in `replay.rs` renders the
  transcript to ONE self-contained HTML file: a metadata header (session id, started/ended,
  distinct models, total cost, total tokens — from the same `summarize()` the text/JSON renderers
  use) then a card per turn (role, model, cost, content, tool calls), monospace, inline CSS only,
  no external assets, dark-friendly (`prefers-color-scheme`). Every dynamic value is HTML-escaped
  — message and tool-call content is untrusted text, not markup. `--html` is single-id only, like
  `--json`.
- **`/export [path]`** — the TUI slash-command counterpart, exporting the CURRENT session with the
  same `render_html`. `path` defaults to `forge-session-<id8>.html` in the current directory.
  Read-only over the persisted record (like `/replay`), so it can run while a turn is in progress.
  Wired in `crates/forge-cli/src/cli/commands/run/export.rs` (a new file — `run/dispatch.rs` is at
  its CI file-size ratchet ceiling, so the dispatch arm just forwards to `export_cmd`).
- This closes the gap named in `docs/architecture/prime-agent-comparison.md` (prime-agent's
  `/export` HTML / `/share` gist vs. Forge's `forge replay` TUI).

## Redaction

Every `forge replay` output (transcript, `--json`, `--html`, two-id diff), `/export` and
`forge anywhere share` masks secrets as `[redacted]` before rendering: configured provider keys,
secret-named env values, credential shapes and `password=…`-style values. `forge replay
--no-redact` prints verbatim. See [redaction.md](redaction.md).

## Re-execution (`--rerun`)

`forge replay <id> --rerun` re-issues the session's recorded **user** prompts (in turn order)
on the *current* model/mesh in a fresh session, then auto-diffs the new run against the
original (summary + per-turn content). This answers "would today's model/config solve this the
same way?" — the reproducibility/audit half of the feature.

- Tools run under the **normal permission mode** (e.g. `--mode bypass` to run unattended), so a
  re-run is no more privileged than re-typing the prompts into `forge run`.
- Only user turns are replayed; the model regenerates its own responses and tool calls.
- Inherently non-deterministic (and costs live tokens), which is why it's opt-in and separate
  from the always-safe read-only inspect/compare path.

The pure prompt-extraction (`replay::user_prompts`) is unit-tested; the orchestration reuses
the same `build_session` + `run_turn` path as `forge run`.

## One entry per answer

A finished turn stores its final answer twice: a provider-continuity `llm_only` row (which carries
the usage) and the published `ui` copy. `Store::load_replay` drops the published copy when it
repeats the row right before it, so `forge replay` and `--html` print each answer once with its
model and cost.

## Turn endings

Every turn that a prompt starts gets a `turn` row (migration 56): opened right after the prompt
is stored, closed in `Session::run_turn_with`, so the TUI, headless `forge run`, the daemon,
subagents and MCP all record it. It holds `end_reason`, `detail_json` (model, open task count,
mutations, error kind), `last_role_shape` (`finished`, `unanswered_prompt`, `pending_tool_calls`,
`unanswered_tool_results`, from the same tail classifier a restart uses) and the start and end
times. Steers and harness nudges join the running turn; they never open one.

End reasons: `final_answer`, `final_answer_mid_intent` (the answer announces a step it did not
take), `final_answer_open_tasks` (it ended with tracked tasks still open), `max_steps`,
`budget_exhausted`, `no_output`, `tasks_unfinished`, `stalled`, `provider_down`,
`daemon_shutdown`, `interrupted` (Esc, a remote interrupt, a queued prompt sent now; written by a
drop guard), `provider_error` (kind in the detail), `no_healthy_model`, `pinned_rejected`,
`review_blocked`, `internal`, `plan_presented`, `daemon_restart_or_crash` (a daemon start closes
turns its predecessor left open) and `abandoned`. `unknown_<shape>` marks backfilled history.

A turn that stops short never ends silently. The core writes one stored note,
`Turn stopped (<reason>): <what to do>` (or `Turn failed: <error>` for an error), unless the
newest visible row already explains the stop. The daemon pushes it as `failed` with the reason
(held pushes escalate once after `push_escalate_secs` even with a client attached), and the
snapshot carries `last_turn_end {reason, label, at}`, which the app and desktop show as a banner.
A prompt that never becomes a turn (a hook block, a rejected input, a failed queued command)
leaves a stored note too, and a prompt sent while the daemon drains joins the stored queue
("queued: runs after the restart"). `forge run --output-format json` adds `end_reason` next to
`stop_reason`.

`forge turns [--since 7d] [--session <id>] [--limit N] [--json]` prints counts by end reason and
the last turns that did not finish; the model report card (`forge models --report`, the Models
page) ends with the same counts. `forge turns --backfill` classifies turns recorded before the
table existed from the transcript tail (shape only, reason `unknown_<shape>`); it prints, and
`--write` stores them.
