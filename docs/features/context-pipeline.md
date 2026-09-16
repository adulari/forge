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
- **Pairing is re-established after fitting.** `fit_messages` keeps the newest suffix that fits
  and strips the orphan tool results at its head; the anchor (the newest user message, kept
  regardless) sorts before that suffix and used to stop the strip early. `to_llm` now also runs
  `normalize_tool_pairs` over the fitted output, so whatever the walk does, no request carries a
  tool result whose call is missing — Moonshot fails the request for one (`tool_call_id  is not
  found`, with no id printed).
- **One ceiling on any tool result.** Before a result enters the transcript it is cut to
  `MODEL_RESULT_MAX_CHARS` (32K chars, ~8K tokens), start and end kept, with a note giving the
  omitted line range and the path of the complete output (the same spool file the full-output
  viewer opens), so the model can `sed -n` the part it needs. Tool-specific caps existed but none
  bounded the whole — `search` returned 272K chars, `read_file` 262K — and elision spares the
  current turn, so each was resent on every step until the turn ended.
- **A bounded working set inside a turn.** Before each request, `age_tool_rounds` cuts every
  tool result older than the last `PRUNE_KEEP_ROUNDS` (4) rounds to a 1,500-char head — but only
  once `AGE_BATCH_ROUNDS` (4) such rounds have piled up, so the prompt prefix (and the provider's
  cache of it) changes once every four rounds, not every step. A cut result names the spooled
  complete output when one exists, so the model reads that file instead of running the tool again.
  Replaying a 30-request Kimi K3 turn: 5.5M chars of tool output sent before, 2.6M after.
- **Search shows every file.** In a directory walk each file contributes at most
  `SEARCH_PER_FILE_CAP` (25) lines plus a count of the rest; a single noisy log no longer fills the
  64 KB budget and hides the file the model was looking for.
- **A large file read whole comes back as a map.** `read_file` without a line range returns a
  file over 24 KB as its size, an outline (`L<start>-<end>  <kind> <signature>` from the Lattice
  tree-sitter extractor, markdown headings otherwise, capped at 16K chars), and its opening 8K
  chars, instead of 256 KB that the result ceiling then cut to two fragments. The model reads the
  span it needs with `start_line`/`end_line`.
- **Accounting honesty**: `estimated_transcript_tokens` (gauge + auto-compaction threshold +
  `transcript_fits`) and the compaction summarizer's rendering both skip `UiOnly` rows — a note
  the model never sees must not trigger compaction or cost summary tokens. The same estimate
  counts a reply's `reasoning`: thinking-mode providers get it back with the message on every
  later call in the loop, so it is prompt the model bills for. Before it was counted, a 200-step
  Kimi K3 turn read ~30K tokens low, and the 80% trigger and window-fit check both fired late.
  A message clipped by `truncate_message_to_budget` drops its reasoning along with its text.

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
