# Feature: context compaction (`/compact` · `/uncompact`)

> **Status: shipped, including auto-trigger, persistence, and undo.** `/compact [what to keep]`
> summarizes the older part of the transcript into one system message written by the session's own
> model, shrinking the live context sent on subsequent turns; it also fires automatically at 80% of
> the context gauge.
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
- The summary is written by the **session's own model** (see "Summary quality" below), not by a
  trivial-tier model: a summary replaces everything it folds, so it is the one side call worth
  spending a capable model on.
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
- **The window has to be known first.** Windows come from the store, which the catalog sweep fills;
  a model with none falls to the 32,000-token conservative floor, so the trigger becomes 25,600.
  A scratch store (`forge bench tasks` gives every task its own `FORGE_DB`) knew no window at all,
  and a run pinned to `inferhub::cb/deepseek-v4.1-flash` (1,048,576 tokens) compacted 13 messages
  down to 7 three or four times per question on a 390k-line repository, clipped tool output and
  trimmed messages from the request. A session now looks up the window of any pinned model nothing
  knows yet (one bounded sweep, at most 6 s, `run/pin_window.rs`) and its first request waits for
  that lookup before it is sized, so the request never goes out on the floor; startup itself does
  not wait (see [turn-latency.md](turn-latency.md)). It says so when the window is still unknown
  afterwards, naming `[mesh.context_windows]`.
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
- **The summary has to be worth keeping.** A reply that is empty, shorter than 600 chars for more
  than 20K chars of transcript, or without the sectioned structure counts as thin: the chain moves
  on (without benching the model), and only when every candidate was thin is the fullest thin reply
  used, with a notice of what was dropped. The structure check counts section markers (at least
  three of the eight numbered sections, or three markdown headings) instead of looking for an
  English word, so a summary in the user's language passes. Kimi's reasoning prefill is not applied
  to tool-less side calls. Observed 2026-09-16: 189 chars stood in for 240K tokens. Side-call usage
  rows name the model (`Store::record_side_call_usage_for`).

### Summary quality (PR A of the context-management program)

Forge's compaction used to lose information for structural reasons: a free model wrote the summary,
it read tool results already cut to 1,500-char heads, an oversized payload lost its middle, and
nothing carried the user's own words across. All four are fixed.

- **The session model writes the summary.** `[mesh] compact_model = "session" | "auto"`, default
  `session`. With `session` the chain is the session model (the `/model` pin set, else the model
  the last turn ran on, else the routed pick for a session that has not run a turn), then its
  fallbacks, then the free `compact_candidates()` shortlist. `auto` is the previous order: free
  shortlist first, the session's model last. Both keep `callable_summarizers` (disabled or unkeyed
  models never receive the transcript) and the foreign-subscription guard (a plan the session is
  not on is never spent on a summary). The call runs at the session's effort (`/effort exact`, the
  `/effort` pin, else the routed rung), not the model's default.
- **Claude Code's structure.** The model first writes an `<analysis>` scratchpad (a chronological
  walk through the requests, actions, decisions, code changes, errors and user corrections), then a
  `<summary>` with eight sections: primary request and intent; key technical concepts; files and
  code (with snippets); errors and fixes (including user corrections); problem solving; pending
  tasks; current work; next step (with a verbatim quote of the latest instruction, only when it
  follows from it). The scratchpad is stripped before anything is stored; a reply without the tags
  is used whole.
- **The user's words, appended by the harness.** After the model's summary Forge adds a
  `## User messages (verbatim)` section built from the folded range's real user messages: the
  newest whole, older ones capped at 2,000 chars, the whole section within ~8k tokens, oldest
  dropped first with the count said. Harness-injected user-role rows (the empty-reply nudge, auto-fix
  and stop-hook feedback, background-job notices, continuation nudges, anything the store marked
  `nudge = 1`) are excluded; steers are the user's own input and stay. Messages are numbered across
  the whole session, so a later compaction reads the earlier block back out of the summary it
  absorbs, appends its own messages and drops from the oldest end. The model never sees the old
  block (the harness re-appends it), so it cannot paraphrase or lose it.
- **Full-fidelity input.** The summarizer reads each tool result as the store holds it (up to
  32,000 chars as the model first saw it) rather than the in-memory pruned head, capped at 12,000
  chars head+tail per result so one giant output cannot dominate. UI-only rows stay out.
- **Rolling summary instead of a dropped middle.** When the folded range exceeds the summarizer's
  input budget (its window minus a 20K-token reserve for the reply, a quarter of a small window at
  most), it is summarized in parts, oldest first: each call receives the summary so far in
  `<previous_summary>` and writes the updated whole. The last-resort per-part guard
  (`fit_compaction_payload`) only cuts a single entry that alone exceeds a window. A failover hop
  recomputes the remaining parts against the next model's window and carries the summary so far.
- **`/compact <focus>`.** Anything after the command (`/compact keep the migration plan and every
  file path`) is appended to the summary prompt for that one compaction, like `/reground`'s
  directive. It does not change the section structure.
- **Bounded call.** The reply is capped at 20,000 tokens (less on a small window); the call has the
  stream-idle watchdog and a 300 s overall limit, and a timeout moves to the next candidate without
  benching a model that may only have been slow.

- **Recent tool rounds survive both passes.** Mid-turn, the prune pass leaves the last
  `PRUNE_KEEP_ROUNDS` (4) tool rounds whole, and a summary keeps those rounds verbatim beside it
  while they cost at most a quarter of the transcript (capped at 40K tokens). Observed 2026-09-16:
  with only 6 messages protected and the check running every step, a Kimi K3 session sitting at
  its 120K ceiling had each step's file reads cut to 1,500 chars before the next step, and read
  the same files again for fifteen minutes. There is no turn-boundary prune any more: finished
  turns are only cut when the elision floor advances (context-pipeline.md).
- **Down to a low-water mark in one go.** When the trigger fires, the prune (a forced elision-floor
  advance) and, if that is not enough, the summary aim for `mesh.compact_low_water` (0.62) of the
  trigger rather than just under it, and a summary that still leaves more than that clips the
  bulkiest results down to half the trigger. Stopping just under the trigger meant the next tool
  result tripped it again, and every trip rewrote the prompt, so a session at its ceiling paid a
  cache miss on nearly every request.
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

### Rehydration after a compaction (PR B of the context-management program)

A summary says what happened; it cannot hold the standing instructions or the text the model was
working from. Before, AGENTS.md, the recalled memories and the task list were ordinary transcript
messages, so a compaction folded them into the summary and nothing restored them. Now:

- **The standing context never enters the transcript.** Project instructions, memories, the goal,
  the approved plan and the task list are the *pinned session context block*, rebuilt for every
  request and placed right after the system prompt (context-pipeline.md, "Standing context is
  pinned"). A compaction cannot fold it away, a resume has it from the first request, and a legacy
  session that still has the old persisted AGENTS.md / memory messages is not sent them twice.
- **Memories are recalled again** at the end of every compaction (`rehydrate_after_compaction`),
  against the goal and the latest real user request still in the transcript, top 6; a resumed
  session recalls on its first turn.
- **The task list snapshot is refreshed** at the same point, so the block carries the list as it
  stood when the summary was written. `update_tasks` returns the full rendered list, which is
  what the model has in hand after a mid-turn compaction, so nothing re-injects the list.
- **Recently used files come back.** The newest five of the last 20 files read or edited are
  re-read from disk and appended as one `[files restored after compaction]` system note (skipping
  a file over 25k tokens, binaries and anything past 40k tokens in total); a bridged claude/codex
  gets the paths only. The note is appended after the store compaction, so the store keeps exactly
  the in-memory tail. The summarizer input leaves such notes out, and only the newest one is sent
  after a later compaction, so copies do not accumulate (`file_restore.rs`, `recent_files.rs`).

### Post-compaction budget and thrash guard

Every part a compaction adds back is budgeted from the effective trigger, the number auto-compaction
compares against (the `[compact_cap]` / free-model cap / 80% of the window, or the room the window
leaves for a request when that is smaller), and capped at the old ceilings. The shares of the
trigger are 18% summary, 14% kept tool rounds and 8% verbatim user messages, with the standing
pinned block taken off first and the total aimed at 50% of the trigger. Restored files get only
what remains under that target. When room is tight the parts give way in this order: restored
files, verbatim older messages, kept rounds, summary. A summary below its old ceiling is told its
size and is cut (middle dropped) at 1.5x it. A big window keeps the old ceilings unchanged.

After a compaction the session remembers the size it left. Auto-compaction does not fire again
until the context has grown by 25% of the trigger past that size. If what is left (the pinned
block and the irreducible tail) already fills the trigger, bulky tool results are clipped instead
of summarizing again, and one notice says so (`compaction_budget.rs`, `compaction_headroom.rs`).
`RUST_LOG=forge_agent_core=info` logs the summarizer chain and why a candidate was skipped or
failed.

### Checkpoint at a landed slice

A long session pays for the whole transcript of the slice it just finished (exploration, superseded
file contents, old tool output) on every later step, until the size trigger finally compacts.
Measured on the owner's store (Sep 1 - Oct 30): 204 commit segments, the 19 over 300 steps used 57%
of all input tokens; 91% of input is cached prompt re-send, the step prompt grows ~1.45k tokens per
step, and 70% of `read_file` calls re-read a path already read. The checkpoint folds a finished
slice at the boundary instead of waiting.

- **When.** A slice has landed when the agent ran `git commit` in this session (exit 0, not
  `--dry-run`) and a check (test, lint, typecheck or build; the same classification as the
  completion gate) passed after the last file edit and before the commit, with no failing check
  outstanding. A later edit un-lands it. The commit is the agent's own: a commit the user made in a
  shell does not count. The decision runs at the next step boundary (`prepare_step_context`), which
  is also the first step of the next turn when the commit was the last thing a turn did.
- **Threshold.** The live transcript must be above `[compact] checkpoint_min_tokens` (default
  60,000); below it the landed slice is dropped without a fold. `[compact] checkpoint = false`
  turns the whole thing off. Both are settable per project (`compact` is in the trusted-safe key
  list).
- **Superseded reads first.** Every `read_file` result of a path that a later call in the slice
  edited is replaced by `[superseded: <path> was read here and changed later in this slice; ...]`
  (results under 400 chars are left). If that alone brings the context under the threshold, nothing
  else folds and the scrollback says `checkpoint: folded the superseded reads behind <sha> (a -> b)`.
- **The fold.** Everything before the assistant message that issued the commit is replaced by one
  summary; the commit round itself stays whole. No model call: the summary is written from facts
  the harness holds. It holds the commit sha, subject and body, the diffstat of `git show --stat`,
  the files the slice edited with a one-line purpose when the file states one (a `//!` line, a
  markdown title or a docstring), the open tasks, the check commands that passed, the shortened
  list of earlier checkpoint slices, a model-written summary from an earlier ordinary compaction
  carried forward unchanged, then the same episode index and verbatim user messages as any
  summary. Recently used files are not restored (the next slice rarely needs them; they are in the
  summary and in `recall`). It installs through the same path as `/compact`, so the store rows are
  soft-deleted (`recall` and `/uncompact` reach them), the persisted view is the compacted one and
  the lifecycle hooks fire with `trigger: "auto"`.
- **Scrollback.** One line on every surface: `checkpoint: folded the slice behind <sha> (192k -> 41k)`
  (token estimates of the transcript before and after).
- **CLI bridge.** Not applicable: a bridged claude/codex keeps its tool rounds in its own process,
  not in the session transcript, so there is nothing for the fold to drop and the tracker (fed by
  direct-path tool outcomes) never sees a commit. Bridge compaction is unchanged.
- Code: `compaction_checkpoint.rs` (tracker, superseded-read fold, summary), `install_fold` in
  `compaction_run.rs` (shared with the model-written compaction), hook in `context_floor.rs`.

### Recall: reaching what a compaction folded away

A compaction used to be a one-way door for the model. The folded rows stay in the store
(`active = 0, compacted = 1`), but only the human's `/uncompact` and `forge replay` reached them,
so a stalled task whose detail had been summarized away could only be asked about or dropped (an
unattended session blocks on the ask or loses the work). Every recorded incident in the long
sessions began after a compaction: the Sep 9 narration stall, the stalled-task redrive, the Sep 16
empty summary.

- **`recall` tool** (`recall.rs`). Read-only, no permission prompt, answers from this session's
  own rows only. `action: "search"` takes `query` (a few words, any order; every word must match,
  falling back to any) and returns up to 12 hits as `seq [role]: snippet`, best first.
  `action: "read"` takes `from_seq` and `to_seq` (default `from_seq + 40`) and returns the folded
  rows in that range, a tool result clipped to 1,500 chars and any other row to 6,000, with a
  `[clipped]` mark, and stops at the character budget with `continue with recall read
  from_seq=N`. A whole result is capped at 8,000 characters. The model sees it only once the
  session has a folded row (`Store::has_folded_history`), so a short session pays nothing, and it
  is a core tool, not deferred behind `tool_search`. One line of `FORGE_SYSTEM` is added only while
  it is advertised. A chat-style question about earlier work ("what did we decide") counts as tool
  intent for a trivial-tier turn (`tool_intent.rs`), otherwise that turn would be sent without it.
- **Both paths.** The direct path dispatches it in `tool_dispatch.rs`; the CLI bridge serves it
  from `forge mcp-serve` (`mcp_serve.rs`), scoped to the parent session through
  `FORGE_CHECKPOINT_SESSION`, listed only when that session has folded rows. Both call
  `forge_core::recall::run`. It is not in the `run_code` catalog (virtual tools are not), though a
  script may still call `tools.recall`.
- **Index** (`recall_store.rs`, migration 57). `message_fts` is an FTS5 table (bundled SQLite,
  porter tokenizer) filled lazily: the first search of a session indexes its folded rows, later
  searches top up whatever is new, and the migration itself scans nothing. Message content never
  changes after it is written, so the index only says which words a row holds; whether a row
  counts as folded is read from `message.compacted` at query time. A fold, `/uncompact`, a rewind
  and a sync apply therefore need no bookkeeping, and an unfolded row simply stops matching. Each
  index row carries the message id, so a rowid SQLite hands out again after a delete cannot match
  the wrong row; deleting a session drops its index rows. Row text is cut at 16,000 chars plus the
  first 2,000 of its tool-call JSON before it is indexed.
- **Episode index** (`compaction_episodes.rs`). Each summary carries `## Earlier episodes (message
  seq ranges)`: one `- seq A-B: <first line of the user prompt that opened it>` per episode, built
  by the harness from the rows being folded (`Store::fold_preview`), so it costs no model output
  and cannot be paraphrased away. At most 12 lines; beyond that the neighbours covering the fewest
  messages merge. A later compaction reads the previous list back out of the summary it absorbs
  (the summarizer never sees it), appends its own episodes and merges again, so the index survives
  summary-of-summaries. It sits before the verbatim section and says to use `recall`.
- **Guidance.** The stalled-task escalation (`task_staleness.rs`) no longer says "ask the user or
  drop it" on its own: after a compaction it tells the model to `recall` (search the task's words,
  read the range) as the one reading step it allows, and to ask or drop only if recall finds
  nothing.
- **For the owner.** TUI `/history <words>` (palette entry; interactive overlay): ranked hits over
  the whole history with a `folded` mark, Enter reads the messages around a hit. Daemon
  `GET /api/sessions/{id}/history/search?q=&limit=` returns `{hits: [{seq, role, snippet,
  folded}]}`; it is a typed `session_history_search` bridge route, and the desktop/mobile session
  menu has a Search history sheet that jumps to the hit. `forge replay <id> --search <words>`
  prints the same hits.

**Deferred**
- Protecting specific messages; configurable keep-count; restoring the
  invoked skill/command guidance and the orchestration/harness one-shots (still stored messages
  that a compaction folds); persisting the approved plan across a resume.

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

Before paying for an LLM summarize, auto-compaction first runs a **free** prune pass: it forces
the elision floor (`context_floor.rs`) forward to the working set and `prune_below()` truncates the
large tool results it released, in place — the file dumps, command logs, and search hits that
dominate context but whose bulk has little value once the turn has moved on. It keeps a head
(`PRUNE_HEAD_KEEP` chars) + a marker, protects the last `PRUNE_KEEP_ROUNDS` tool rounds, the newest
`tool_result_context_keep_recent` results and the last `COMPACT_KEEP_RECENT` messages, only touches
`Tool` results over `PRUNE_TOOL_RESULT_MAX`, and is idempotent. The full text stays in the store for replay — only the model-facing transcript is
trimmed.

`auto_compact_if_needed()` prunes first and re-checks `transcript_fits`; the expensive summarize
only runs if pruning didn't reclaim enough. On a tool-output-heavy session this avoids the
summarize round-trip (and its model cost) entirely. (Adopted from opencode's `compaction.prune`; see
`docs/harness/competitor-gap-analysis.md`.)

## 4. Design
`Session::compact()` (forge-core, `compaction_run.rs`): splits the transcript at the round-aligned
kept tail, builds the summarizer input from the folded range (`compaction_input.rs`: stored tool
results, harness-nudge detection), walks the candidate chain (`compaction_chain.rs`) calling
`provider.complete_with` once per part with the system prompt in `compaction_summary.rs`, strips
the scratchpad, appends the verbatim user messages, then sets `transcript = [system summary,
...recent]`, then rehydrates (memory recall, task snapshot, restored files). Returns `(before, after)` and emits a `Warning` note. `/compact` → `CommandAction::Compact` →
`DispatchOutcome::RunCompact` → `spawn_compact` (background task, busy/done machinery), gated
while a turn is in flight.

## 5. Definition of done
- [x] `Session::compact()` folds older → summary, keeps recent, no-op when short.
- [x] Session-model summary call (`[mesh] compact_model`); structured prompt with a hidden scratchpad.
- [x] Verbatim user messages, full-fidelity input, rolling parts, `/compact <focus>`, 20K output cap and timeout.
- [x] `/compact` command + palette entry; runs as a background task.
- [x] Unit tests (fold + no-op); `cargo fmt` + `clippy -D warnings` clean.
- [x] Auto-trigger on gauge threshold (80%); zero-LLM prune pass first.
- [x] Persist across resume (`message.active` soft-delete + `session_compaction` summary row).
- [x] `/uncompact` undo (#471) — store + session tests, live TUI e2e.
