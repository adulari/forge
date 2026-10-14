# Feature: two-phase context pipeline — one seam between transcript and provider

> **Status (shipped):** `forge_core::context_pipeline` (gap-analysis #9). Phase 1
> `context_floor::prune_below` mutates the transcript in place, only below the elision floor
> (zero-LLM reclaim of old tool output). Phase 2 `to_llm_floored(&[Message], budget, …, floor,
> &mut trim)` is the pure per-request view: strip `Visibility::UiOnly` messages, shorten old output
> below the same floor, then window-fit what remains. `Message.visibility`
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
- **Phase 1 — `prune_below`**: the mutating transform, run only when the elision floor advances
  (see "One floor that only moves forward" below).
- **Phase 2 — `to_llm_floored`**: called by `transcript_for` / `transcript_with_preamble_and_tools`,
  i.e. every main-loop provider request. Pure: filter `UiOnly`, normalise system context, pair tool
  calls, collapse repeats and elide bulky results below the floor, then `fit_sticky` (system
  messages and the newest user message always kept).
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
- **One floor that only moves forward** (`context_floor.rs`). Provider prompt caches key on a
  prefix, so rewriting an already-sent message re-bills everything after it. Old tool output used
  to be shortened from four places on four schedules — a prune at every user turn, an elision
  boundary that jumped to each new turn, a dedupe window counted from the tail (which collapsed a
  result a few results after sending it verbatim, mid-turn) and an every-step aging pass that cut
  rounds every ~4 steps. Now there is one `Session.elision_floor`, a transcript index: in-place
  pruning, request-view elision and dedupe all act only below it, and old turn contracts are only
  dropped below it. While it stands still every request is a pure append to the last one — a
  randomized test (`an_unchanged_floor_makes_the_view_append_only`) checks that `to_llm(T)` is an
  element-wise prefix of `to_llm(T + E)` for any transcript and extension.
  - It advances before each request, in blocks: only when more than
    `max(mesh.context_floor_block_tokens (32768), 25% of the compaction trigger)` tokens sit
    verbatim above it AND the move releases at least half a block (the hysteresis — otherwise a
    working set larger than a block would move it one round per step). It moves to just before the
    working set: the last `PRUNE_KEEP_ROUNDS` (4) rounds, the newest
    `tool_result_context_keep_recent` results and the last 6 messages, snapped back to a user or
    assistant message. Under context pressure auto-compaction forces the move.
  - It never moves backward within a transcript epoch; a compaction, rewind or reload starts a new
    epoch, where it is recomputed as "before the current turn" (the old elision boundary).
  - A released result is cut to a 1,500-char head that names the spooled complete output when one
    exists, so the model reads that file instead of running the tool again.
- **A sticky window trim.** When the view no longer fits the window, `fit_sticky` drops the oldest
  messages down to 85% of the budget once and keeps that cut while the result still fits, instead
  of re-deriving a newest-first fill (a new prefix) on every call. A smaller window after failover
  fails the fit check and re-fits.
- **System context depends only on what came before.** `normalize_system_context` keeps the first
  copy of repeated guidance plus any repeat directly after an assistant reply (a harness reaction);
  it no longer looks at whether a user prompt follows. Turn contracts stay where they were
  injected: a turn whose contract differs appends `Turn contract: supersedes earlier turn
  contracts, which no longer apply — …`, including a turn with no explicit contract after one that
  had one ("this request sets no special contract"), so a stale "explicitly read-only" contract is
  never the last word. An unchanged contract is not repeated.
- **Per-turn context says only what changed.** The git branch moved out of the `<env>` preamble
  (where a switch rewrote the first message of every request) into an `[env] git_branch: …` system
  message appended when it differs from the last one in the transcript. The full clock instruction
  goes out on the first turn only; later turns get `[time] <timestamp>`. A session that has
  advertised tools once keeps advertising them on trivial turns even after compaction folded away
  every tool message (`tools_advertised_once`).
- **Cache instrument** (`cache_instrument.rs`, `forge_provider::wire_fingerprint`). Every
  main-loop request is hashed per message (SHA-256, truncated) after the provider-side transforms —
  system placement and the latest-image-turn rule — plus one hash of the tool list, and compared
  with the previous request to the same model. The usage row records `prefix_reused_msgs`,
  `prefix_total_msgs`, `tools_changed` and a `cache_label` (store migration 41): `append`,
  `harness_rewrite@<idx>(<kind>)`, `floor_advance`, `age_rounds`, `compaction` (a summary, or the
  post-compaction clip of bulky results), `history_rewrite` (a rewind or a full-history reload),
  `tools_changed`, `model_switch`, `cold`, `provider_eviction` (prefix intact, under half the input cached) or
  `n/a` for CLI bridges, which report cumulative usage. Two `harness_rewrite` calls in a row raise
  a warning in the TUI. `forge stats cache [--session [ID]] [--days N]` shows the hit rate per
  model, the dollars the uncached input cost over cache-read pricing, and the misses by label.
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
| `forge-core/src/context_pipeline.rs` | `to_llm_floored`, moved `fit_messages`/`prune_tool_results`/`message_tokens` + tests |
| `forge-core/src/context_floor.rs` | the elision floor, block advance, `prune_below`, `fit_sticky` |
| `forge-core/src/cache_instrument.rs` | per-request prefix comparison, miss labels, rewrite-streak warning |
| `forge-core/src/turn_context.rs` | turn contract / clock / git branch appended only when changed |
| `forge-provider/src/wire_fingerprint.rs` | per-message wire hashes after system placement + image stripping |
| `forge-store` | migration 41: `usage.prefix_reused_msgs`, `prefix_total_msgs`, `tools_changed`, `cache_label`; `cache_stats` |
| `forge-cli` | `forge stats cache` |
| `forge-core/src/lib.rs` | call sites rewired; gauge/compaction skip UiOnly; error notes persisted via `add_ui_note` + tagged `ui_only()` |
| `forge-core/src/subagent.rs` | transcript rebuild preserves visibility |
| `forge-store` | schema v7, `migration_0007`, `StoredMessage.visibility`, `add_ui_note`, fork copy |
