# Feature: context compaction (`/compact` · `/uncompact`)

> **Status: shipped, including auto-trigger, persistence, and undo.** `/compact` summarizes the
> older part of the transcript into one system message via a cheap model call, shrinking the live
> context sent on subsequent turns; it also fires automatically at 80% of the context gauge.
> `/uncompact` reverses it — the full transcript is always recoverable. Pairs with the
> context-window gauge (tui-token-counter.md).

## 1. Problem (JTBD)
> When a session gets long, I want to fold the early history into a summary so I stop paying to
> resend it every turn and don't overflow the model's context window — without losing the
> decisions and facts that matter.

The gauge surfaces the fill level; compaction is the action that lowers it.

## 2. Scope (MoSCoW)
**Must have (shipped)**
- `/compact` (TUI command + palette entry) summarizes all but the most recent
  `COMPACT_KEEP_RECENT` (6) messages into a single `Role::System` summary, prepended ahead of
  the kept tail. No-op when there are fewer than `KEEP_RECENT + COMPACT_MIN_OLDER` messages.
- The summary is produced by one **trivial-tier** model call (cheap, mesh-routed) with a fixed
  system prompt that preserves decisions, facts, file paths, names, and open threads.
- Runs as a **background task** like a turn (the spinner ticks; doesn't block the render loop).

**Shipped since the MVP**
- **Auto-trigger**: `auto_compact_if_needed()` runs when the context gauge crosses 80% —
  prune pass first (§3a), summarize only if pruning didn't reclaim enough. The trigger is
  `min(0.8 × window, ceiling)`, where the ceiling is `mesh.compact_cap_tokens` (217,600) for a
  paid model, `mesh.free_model_cap_tokens` for a free one, or a `[compact_cap]` entry for the
  model or its provider when one exists. `[compact_cap]` ships with `kimi = 120000`: Kimi Code
  meters by request, its K3 window is 262K, and its replies carry thinking that is sent back on
  every step, so a measured 198-step session averaged a 130K prompt under the global ceiling.
  A lower ceiling there roughly halves that at the cost of summarising more often; set the
  provider to `0` to go back to the global ceiling.
- **An empty summary is never committed.** The summary replaces every message it folds, and the
  `session_compaction` row is upserted, so an empty one would erase the history *and* the previous
  real summary. Observed 2026-09-16: a summarizer answered 52K tokens of transcript with 13 tokens
  of nothing and the session forgot its task. `compact()` now treats a blank reply like a failed
  call and walks the candidate chain — without benching the model, since the session's own model
  is last in that chain — and returns an error with the transcript untouched when every candidate
  is blank. `Store::compact_session_store` refuses an empty summary independently.
- **The kept tail never starts inside a tool round.** The split is `round_aligned_split`: if the
  first kept message is a tool result, the split walks back to the assistant call that produced
  it, so a few more than `COMPACT_KEEP_RECENT` messages survive rather than results whose call
  is in the summary (Moonshot rejects that request; other providers lose the results silently).
- **The summary has to be worth keeping.** `COMPACT_SYSTEM` asks for a sectioned working memory
  (goal, current state, files and code, findings and decisions, errors, next steps) sized to the
  conversation, and the call runs at the model's default effort instead of the cheap rung. A reply
  under 600 chars for more than 20K chars of transcript counts as thin, like an empty one: the
  chain moves on, and only when every candidate was thin is the fullest thin reply used. Kimi's
  reasoning prefill is not applied to tool-less side calls, and the summary reserve is up to 6K
  tokens (an eighth of a small window). Observed 2026-09-16: 189 chars stood in for 240K tokens.
  Side-call usage rows now name the model (`Store::record_side_call_usage_for`).
- **Recent tool rounds survive both passes.** Mid-turn, the prune pass leaves the last
  `PRUNE_KEEP_ROUNDS` (4) tool rounds whole, and a summary keeps those rounds verbatim beside it
  while they cost at most a quarter of the transcript (capped at 40K tokens). Observed 2026-09-16:
  with only 6 messages protected and the check running every step, a Kimi K3 session sitting at
  its 120K ceiling had each step's file reads cut to 1,500 chars before the next step, and read
  the same files again for fifteen minutes. The turn-boundary prune still trims finished turns
  down to the last 6 messages.
- **Checked every step.** `auto_compact_if_needed` runs before every model request in the tool
  loop, not only at nudge/guard points. Observed 2026-09-16: a single Kimi turn grew 62K → 246K
  tokens under a 120K ceiling because nothing mid-turn asked.
- **Persistence**: compaction is durable across resume. Compacted messages are soft-deleted
  (`message.active = 0`) and the summary stored as a `session_compaction` row; `load_messages`
  reloads the compacted view, while the full history stays intact underneath.
- **Undo — `/uncompact` (#471)**: restores the full pre-compaction transcript. One immediate
  transaction reactivates the messages and drops the summary row
  (`Store::uncompact_session_store`), then `Session::uncompact()` reloads the live transcript
  and reports `before → after`. A no-op with a note when the session was never compacted.

**Deferred**
- Pinning/protecting specific messages; configurable keep-count; summary-of-summaries.

## Non-goals
- No change to cost math or the agent loop. Compaction reshapes what the next turn sends (and
  which store rows are active) — it never deletes history: the full transcript stays in the store
  and `/uncompact` or `forge replay` can always reach it.

## 3. Acceptance criteria
```
Given a transcript longer than KEEP_RECENT + COMPACT_MIN_OLDER
When /compact runs
Then the older messages become one system summary, the recent KEEP_RECENT are kept verbatim,
 and transcript length drops to KEEP_RECENT + 1

Given a short transcript
When /compact runs
Then it is a no-op (no model call, length unchanged)

Given /compact is invoked
When it runs
Then it runs in the background (spinner animates) and emits a "compacted N → M" note
```

## 3a. Zero-LLM prune pass (auto-compaction fast path)

Before paying for an LLM summarize, auto-compaction first runs a **free** prune pass:
`prune_tool_results()` (forge-core) truncates large **old** tool results in place — the file dumps,
command logs, and search hits that dominate context but whose bulk has little value once the turn
has moved on. It keeps a head (`PRUNE_HEAD_KEEP` chars) + a marker, protects the most recent
`COMPACT_KEEP_RECENT` messages, only touches `Tool` results over `PRUNE_TOOL_RESULT_MAX`, and is
idempotent. The full text stays in the store for replay — only the model-facing transcript is
trimmed.

`auto_compact_if_needed()` prunes first and re-checks `transcript_fits`; the expensive summarize
only runs if pruning didn't reclaim enough. On a tool-output-heavy session this avoids the
summarize round-trip (and its model cost) entirely. (Adopted from opencode's `compaction.prune`; see
`docs/harness/competitor-gap-analysis.md`.)

## 4. Design
`Session::compact()` (forge-core): splits the transcript at `len - COMPACT_KEEP_RECENT`, renders
the older messages as `role: content` text, routes a trivial-tier model
(`route_hinted(..., Some(Trivial))`), calls `provider.complete` once with a fixed
summary system prompt, then sets `transcript = [system summary, ...recent]`. Returns
`(before, after)` and emits a `Warning` note. `/compact` → `CommandAction::Compact` →
`DispatchOutcome::RunCompact` → `spawn_compact` (background task, busy/done machinery), gated
while a turn is in flight.

## 5. Definition of done
- [x] `Session::compact()` folds older → summary, keeps recent, no-op when short.
- [x] Trivial-tier model call; fixed information-preserving prompt.
- [x] `/compact` command + palette entry; runs as a background task.
- [x] Unit tests (fold + no-op); `cargo fmt` + `clippy -D warnings` clean.
- [x] Auto-trigger on gauge threshold (80%); zero-LLM prune pass first.
- [x] Persist across resume (`message.active` soft-delete + `session_compaction` summary row).
- [x] `/uncompact` undo (#471) — store + session tests, live TUI e2e.
