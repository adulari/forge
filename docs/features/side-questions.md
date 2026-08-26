# `/btw` — side questions that never touch the session

> Status: **done** — `/btw <question>` (alias `/side`) in the TUI. Ported from prime-agent's
> `/btw`/`/side`, ranked in `docs/architecture/prime-agent-comparison.md`.

## Why

Sometimes you want to ask the assistant something *adjacent* to the current task — "what does
EAGAIN mean", "is this worth a Jira ticket" — without derailing the session: the question and its
answer are irrelevant to the coding task at hand, and letting them into the transcript pollutes
every subsequent turn's context (and the eventual `/compact` summary) with off-topic noise.

`/btw` answers the question out of band: it runs one model call, shows the answer as a distinct
side-note card, and then forgets it completely. The main conversation — transcript, context,
cost accounting via `store.add_message`/`record_side_call_usage` — never sees it.

## What shipped

- **`/btw <question>`** (alias **`/side`**) — routes one cheap trivial-tier model call through the
  mesh (mirrors `/compact`'s `TaskTier::Trivial` routing and `auxiliary_policy.rs`'s existing
  "side call" shape: recap, suggest, memory-capture, shell-diagnose) and renders the answer as a
  side-note card (`◈ btw …`) in the TUI scrollback — visually and structurally distinct from an
  assistant message.
- **Never persisted, never joins history.** `Session::ask_btw` (`crates/forge-core/src/
  btw_policy.rs`) calls `self.provider.complete_with(...)` directly. It never pushes to
  `self.transcript` and never calls `self.store.add_message` or `record_side_call_usage` — not
  even the inactive anchor row the latter would otherwise write. A fresh session's message table
  has zero rows before and after a `/btw` call; see
  `ask_btw_writes_nothing_to_the_message_table` in `forge-core/src/lib.rs`'s test module.
- **Best-effort, like the rest of the side-call family.** Budget exhaustion, no available
  non-bridge model, or a provider error all emit a `PresenterEvent::Warning` instead of failing
  the session — `/btw` can never break the main turn loop.
- Bridged CLI models (`claude-cli::…`, `codex-cli::…`) are skipped for `/btw` the same way they
  are for recap/suggest (`post_turn_auxiliary_model`) — launching a whole subscription-CLI
  subprocess for a one-line side question isn't worth the latency.

## Deliberate divergence from prime-agent: no side-conversation state

prime-agent's `/btw` keeps a running side-conversation — follow-up `/btw` calls see each other's
history, separate from (but persistent alongside) the main thread.

Forge's `/btw` does **not**. Each call is completely independent: no prior `/btw` question or
answer is visible to the next one. This is a deliberate simplification, not an oversight:

- It keeps the "never touches the store" guarantee airtight — a side-conversation needs *some*
  place to live, and the moment it's persisted anywhere it stops being obviously safe to drop.
- It keeps the mental model simple: `/btw` is a single question-answer pair, not a second session
  you have to remember you're in.
- If a side question turns out to matter enough to want follow-ups, the answer is to promote it
  into the main conversation (just ask it there) rather than grow a parallel thread.

This may be revisited if real usage shows the statelessness is actually painful — tracked as a
possible follow-up, not planned work.

## Design

- **Logic:** `crates/forge-core/src/btw_policy.rs` — `Session::ask_btw(&mut self, question: &str)`.
  Same shape as `auxiliary_policy.rs`'s `diagnose_shell_error`: build a `BudgetState`, route via
  `self.router.route_hinted(question, false, budget, &health, &quota, Some(TaskTier::Trivial),
  self.pinned_effort, &self.project)`, resolve a non-bridge model via `post_turn_auxiliary_model`,
  then `self.provider.complete_with(&model, &messages, &[], &completion_opts, &mut on_event)`.
- **Event:** `PresenterEvent::BtwAnswer { question, answer, model, cost_usd }`
  (`crates/forge-types/src/interaction.rs`) — a new variant alongside `ShellDiagnosis`/`Recap`,
  carrying enough for the card without ever being written anywhere.
- **Rendering:** `App::apply` in `crates/forge-tui/src/app.rs` turns a `BtwAnswer` event into a
  `◈ btw …` card via `btw_answer_lines` (same style family as `shell_diagnosis_lines`) — question,
  answer body, then a dim `(model · $cost, not part of this session's history)` footer so it's
  unambiguous the exchange won't be remembered.
- **TUI command registration:** parsed in `crates/forge-tui/src/commands/btw_args.rs` (a new
  submodule — `commands.rs` sits at its CI file-size ratchet ceiling, so new command logic lives
  in its own file and the `parse_command` match arm stays one line:
  `"btw" | "side" => CommandAction::Btw(btw_args::parse_btw_arg(&arg))`).
- **Dispatch:** `/btw` makes a model call, so it's spawned as a background task exactly like
  `/compact` — `DispatchOutcome::RunBtw { question }` (`run/dispatch.rs`) →
  `spawn_btw` (`crates/forge-cli/src/cli/commands/run/btw.rs`, a new file for the same
  ratchet-ceiling reason) → `Session::ask_btw`. It's excluded from the "safe while a turn is busy"
  exemption list (like `/compact`, unlike `/replay`) since it needs the session lock to reach the
  provider/router.

## Testing

- `crates/forge-tui/src/commands/btw_args.rs` — arg parsing (trim, empty stays empty).
- `crates/forge-tui/src/commands.rs` (`parses_new_commands`) — `/btw`, `/side` alias, bare
  `/btw`/`/side`, and `/export` parse forms.
- `crates/forge-core/src/lib.rs` (`mod tests`) —
  `ask_btw_writes_nothing_to_the_message_table` (the airtight guarantee, checked via
  `Store::load_all_messages`, which — unlike `message_count` — includes soft-deleted rows too, so
  it would also catch a future accidental `record_side_call_usage` call),
  `ask_btw_emits_a_btw_answer_event_not_a_transcript_message`, and
  `ask_btw_on_blank_question_warns_and_makes_no_call`.
