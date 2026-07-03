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
  prune pass first (§3a), summarize only if pruning didn't reclaim enough.
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
