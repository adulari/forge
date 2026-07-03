# Feature: two-phase context pipeline — one seam between transcript and provider

> **Status (shipped):** `forge_core::context_pipeline` (gap-analysis #9). Phase 1
> `prune_and_inject(&mut [Message], keep_recent)` mutates the transcript at turn boundaries
> (today: zero-LLM reclaim of old tool output; the designated home for future injections).
> Phase 2 `to_llm(&[Message], budget_tokens)` is the pure per-request view: strip
> `Visibility::UiOnly` messages, then window-fit what remains. `Message.visibility`
> (`Llm` default | `UiOnly`) is persisted in the store (`message.visibility`, migration 0007,
> schema v7) and carried across resume, forks (`fork_session` copies the tag), and the
> subagent transcript rebuild.

## 1. Problem

Forge injects a growing pile of context around the user's words — AGENTS.md, recalled memories,
Lattice retrieval, skill guidance, hints — and also persists user-facing *notes* (turn-ending
budget-stop / no-usable-model errors) as ordinary system messages. Those notes are for the human:
after a resume they re-entered the prompt as stale harness chrome, inflated the token gauge, and
were even paid for in compaction summaries. And with every new injection site, "what exactly does
the model see?" was answered in more places.

## 2. Design

- **`Visibility { Llm, UiOnly }` on `Message`** (forge-types), serde-default `Llm` so every
  existing constructor and stored row is unchanged. `Message::ui_only()` opts a message out of
  the model's view; `Store::add_ui_note` persists it with `visibility='ui'`.
- **Phase 1 — `prune_and_inject`**: the mutating transform, run where the transcript itself must
  change (auto-compaction's cheap pre-pass). Anything that should *survive* in the transcript
  belongs here, not scattered across call sites.
- **Phase 2 — `to_llm`**: called by `transcript_for` / `transcript_with_preamble`, i.e. every
  main-loop provider request. Pure: filter `UiOnly`, then `fit_messages` (system messages always
  kept, newest-first fill, orphan-tool-result demotion) — which moved into the module wholesale.
- **Accounting honesty**: `estimated_transcript_tokens` (gauge + auto-compaction threshold +
  `transcript_fits`) and the compaction summarizer's rendering both skip `UiOnly` rows — a note
  the model never sees must not trigger compaction or cost summary tokens.

## 3. Persistence

`message.visibility TEXT NOT NULL DEFAULT 'llm'` in the base schema AND `migration_0007`
(`add_column_if_missing`, idempotent on fresh DBs), `SCHEMA_VERSION = 7`. Read paths
(`load_messages`, `load_all_messages`) return it on `StoredMessage`; `Session::resume`,
`reset_resumed`, `reload_full_context`, and the subagent rebuild map it back onto the live
transcript; `fork_session` copies it so a UI note in a fork's prefix stays UI-only.

## 4. Invariants

- A `UiOnly` message never reaches any provider, in any session lineage (fresh, resumed,
  forked, subagent follow-up).
- UI notes still render everywhere the user looks: scrollback, `forge replay`, `load_all_messages`.
- The gauge, `transcript_fits`, and compaction cost reflect only what a model can actually see.

## 5. Surfaces touched

| Layer | Change |
|---|---|
| `forge-types` | `Visibility` enum + `Message.visibility` + `Message::ui_only()` |
| `forge-core/src/context_pipeline.rs` | new module: `prune_and_inject`, `to_llm`, moved `fit_messages`/`prune_tool_results`/`message_tokens` + tests |
| `forge-core/src/lib.rs` | call sites rewired; gauge/compaction skip UiOnly; error notes persisted via `add_ui_note` + tagged `ui_only()` |
| `forge-core/src/subagent.rs` | transcript rebuild preserves visibility |
| `forge-store` | schema v7, `migration_0007`, `StoredMessage.visibility`, `add_ui_note`, fork copy |
