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
- **The trim protects the task, not just the newest user message.** `fit_sticky` (and the
  last-resort `fit_messages_owned`) always keep the system messages and every *real* user message
  of the current turn (`context_floor::turn_protected`): the originating request plus any steers
  queued into it. A turn starts at the newest real user message that does not follow a tool result,
  an assistant message with calls, or a nudge; a message that does is a steer, and the walk
  continues to the one before it. If those alone exceed the budget they are clipped from the front,
  newest kept whole first, never dropped (each keeps at least a 64-token tail).
- **Harness messages are tagged, not guessed.** `Message.kind` (`forge_types::MessageKind`,
  persisted in the row's `nudge` column: 0 normal, 1 user-role nudge, 2+ ephemeral note kinds)
  marks user-role nudges (empty-reply, empty-diff, test-edit guard, stop hook, auto-fix failures)
  as `Nudge`, so `Message::is_real_user` is false for them: they never anchor a turn, never set
  `current_turn_seq`, the floor's turn start, the review gate's "latest request" or the
  deterministic digest. The tag survives a resume (`StoredMessage.kind`).
- **Ephemeral notes are superseded.** Clock lines (`Clock`), tool-result hints (`Hint`), and the
  silence / loop-guard / continue / deadline nudges carry their kind; `normalize_system_context`
  keeps only the newest *run* (consecutive notes of one kind, i.e. one step's hints) of each kind
  below the floor, under the same cache rule as contracts: a newer run above the floor never
  reaches back, so the cached prefix is rewritten only when the floor moves. Notes without a kind —
  skill/command guidance, turn contracts, the full clock instruction — are never touched. A
  `FilesRestored` note (the file text re-read after a compaction) is the exception to the floor
  rule: only the newest copy is ever sent, because a new one appears together with a compaction,
  which rewrote the prompt anyway.
- **Standing context is pinned, not injected** (`pinned_context.rs`). The project instructions
  (first non-empty of `.forge/AGENTS.md`, `AGENTS.md`, `CLAUDE.md`, `.claude/CLAUDE.md`, then the user-global `<config>/forge/AGENTS.md`, `~/.forge/AGENTS.md`, `~/.claude/CLAUDE.md`), the memories recalled for the session, the active goal, the
  approved plan and the task list used to be ordinary system messages injected once, so a
  compaction folded them into the summary and nothing put them back. They are now one system
  message of kind `Pinned`, built fresh for every request (`Session::pinned_context_message`) and
  placed right after the system prompt, env block and codemode guidance in
  `transcript_with_preamble_and_tools` (and at the head of `transcript_for`, which the planner and
  the finalizer read). It is never stored and never in the in-memory transcript, so it can be
  neither folded nor duplicated, and its tokens are subtracted from the window budget and counted
  in `estimated_transcript_tokens`. Its inputs:
  - **Project instructions**: `Session::cached_agents_md`, read at construction (resumed sessions
    too) and re-read when the file's (path, mtime, length) fingerprint moves, checked with one
    `stat` per turn (`refresh_project_instructions`). A bridged `claude-cli` leaves them out when
    its own `CLAUDE.md` already carries them (`lean_context::claude_md_covers`).
  - **Memories**: recalled with the goal plus the person's request (top 6) on a session's first
    turn, on the first turn after a resume, and again after every compaction
    (`recall_pinned_memories`; the compaction recall uses the latest real user request still in
    the transcript). Auto-memory off leaves the section out. The query embedding starts before
    routing (`prefetch_recall_embedding`) so its round trip overlaps the route, and a store with
    no project or global memories embeds nothing. An embedding model that rejects a request (a
    4xx other than a rate limit, e.g. the retired `gemini::text-embedding-004`) is not asked again
    in that process; recall and hybrid code retrieval take their keyword/structural path.
  - **Goal**: `Session::set_active_goal`, set by `/goal` and cleared when the goal run stops.
  - **Plan**: the plan the person approved (`activate_plan_tasks`); in memory only.
  - **Task list**: a snapshot taken at the start of each turn and after each compaction, not live.
    A live list would rewrite the block, and with it the provider's cached prefix for the whole
    conversation behind it, on every `update_tasks` call. `update_tasks` returns the full rendered
    list instead (`- [status] title` per task), so the newest state is in the conversation where
    the model made it, and the block's heading says that a later `update_tasks` result is newer.
    The per-turn `[task list]` note keeps its instruction (apply the new message to the list,
    do not start over) but no longer quotes the list a second time.
  - **Legacy sessions**: a session stored before the block existed carries the AGENTS.md text and
    the memory block as persisted system messages. While the transcript still holds the exact
    AGENTS.md body, or a message opening with the memory header, the block leaves that section
    out; once a compaction folds the stored copy away, the block carries it.
  - **Cache**: the block follows the stable prefix, so `forge_provider::cache_hints` marks a
    breakpoint at the end of the prompt/env prefix, one at the end of the block and one on the
    last message; the block changing re-bills the block and the conversation, not the prompt. It
    changes only when AGENTS.md is edited, a recall finds different memories, the goal or plan
    changes, or a turn starts or a compaction runs with a different task list.
  - **Bridges**: `render_prompt` puts the block in the system preamble of a fresh session. A resume
    delta carries only what follows the leading system run, so `ResumeState`/`LiveSession` keep a
    fingerprint of the block they last sent and the delta opens with `Updated session context
    (replaces the earlier version): …` when it differs; unchanged, it is not sent again.
- **Files come back after a compaction** (`recent_files.rs`, `file_restore.rs`). The session keeps
  the 20 files it read (`read_file` `path`/`file_path`/`paths`, line ranges stripped) or edited
  (the write tools' path) most recently, newest first. Right after a successful compaction the
  newest five that still exist on disk are re-read and appended as one system note of kind
  `FilesRestored` (`[files restored after compaction]`, one `===== path =====` block per file,
  verbatim like `read_file`): a file over 25k tokens, a binary or non-UTF-8 file, or one that would
  pass the 40k-token total is skipped, not cut. A bridged `claude-cli`/`codex-cli` session only
  gets the paths (up to 10), since the CLI reads files itself. The note is stored, so a resume has
  it; the summarizer input leaves it out; a later compaction's note replaces it in the request.
- **A trim that would drop history compacts first, and says so.** After the per-step
  `auto_compact_if_needed`, `request_provider_response` checks what the request build trimmed
  (`TrimDrop`): if user or assistant messages were dropped it runs a compaction and rebuilds the
  request (at most 3 per turn, and not again once one folded nothing). Whatever is still trimmed
  raises one `context window full: N older message(s) are left out of this request` warning per
  turn.
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

The `nudge` column doubles as the message kind (`MessageKind::code`); nothing else reads it as
more than "non-zero", and the history page still reports only user-role rows as nudges.
Compaction persists the tail the live transcript kept: `compact_session_store` is given the
round-aligned tail length, not a fixed 6, so a resume reproduces the post-compaction transcript
and never opens on an orphaned tool result. `reload_full_context` (`/uncompact`, "continue
without compaction") reads `load_messages_uncompacted`: compacted rows return, `/undo`-rewound
rows do not. A compaction that gives up after `CompactionStarted` emits `CompactionFinished`
with `before == after`, so the TUI band clears. The CLI's idle auto-compact uses
`Session::idle_compaction_trigger` (the pinned or last-run model's `compaction_trigger`), so
`[compact_cap]` and the free-model ceiling apply, and it reports the compaction as automatic.

The CLI bridges track what they sent by identity: `ResumeState`/`LiveSession` keep a `SentMark`
(fingerprints of the last two sent messages) next to the count, and the resume delta starts after
the marked message wherever the front-trimmed view now has it; an ambiguous or missing mark means
a fresh session instead of a skipped message.

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
| `forge-core/src/context_floor.rs` | the elision floor, block advance, `prune_below`, `fit_sticky`, `turn_protected`, trim watch |
| `forge-core/src/harness_notes.rs` | `push_note` / `push_nudge`: tagged, persisted harness messages |
| `forge-core/src/system_context.rs` | supersede ephemeral notes below the floor; newest `FilesRestored` only |
| `forge-core/src/pinned_context.rs` | the pinned session context block, memory recall, task/plan rendering |
| `forge-core/src/recent_files.rs`, `file_restore.rs` | recently used files; re-read and restore after a compaction |
| `forge-types` | `MessageKind::Pinned` (never stored), `MessageKind::FilesRestored` |
| `forge-provider/src/cache_hints.rs` | breakpoint behind the stable prefix, before the pinned block |
| `forge-provider/src/cli_provider/resume.rs` | `SentMark`, `locate_sent`, pinned-context fingerprint and update |
| `forge-core/src/cache_instrument.rs` | per-request prefix comparison, miss labels, rewrite-streak warning |
| `forge-core/src/turn_context.rs` | turn contract / clock / git branch appended only when changed |
| `forge-provider/src/wire_fingerprint.rs` | per-message wire hashes after system placement + image stripping |
| `forge-store` | migration 41: `usage.prefix_reused_msgs`, `prefix_total_msgs`, `tools_changed`, `cache_label`; `cache_stats` |
| `forge-cli` | `forge stats cache` |
| `forge-core/src/lib.rs` | call sites rewired; gauge/compaction skip UiOnly; error notes persisted via `add_ui_note` + tagged `ui_only()` |
| `forge-core/src/subagent.rs` | transcript rebuild preserves visibility |
| `forge-store` | schema v7, `migration_0007`, `StoredMessage.visibility`, `add_ui_note`, fork copy |
