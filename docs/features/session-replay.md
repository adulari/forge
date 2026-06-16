# Session replay — auditable, reproducible runs

> Status: **MVP done** — `forge replay <id>` (transcript) + `forge replay <a> <b>` (diff),
> read-only over the persisted record. True model re-execution is deferred.

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

## Deferred

- **True re-execution** — re-issue the recorded prompts against the recorded model versions
  and diff the *new* output vs. the recorded one. Non-deterministic and needs live keys, so
  it can't be verified offline; the inspect/compare half is the verifiable, high-value 80%.
- Per-turn aligned content diff (not just summary-level), a `/replay` chat command, and JSON
  export for external auditing.
