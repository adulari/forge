# Feature: `/rewind` — one command that rewinds chat, context and files together

> `/rewind` (alias `/undo`, or press **Esc twice** while idle) opens the message picker; choosing a
> message opens an action menu: restore code and chat (rewind the conversation to just before it,
> put back the files those turns wrote, and put the message back in the input box), chat only,
> code only, or summarize from there. Touches `forge-core` (`session_history.rs`,
> `session_rewind.rs`), the CLI
> bridge (`forge-provider/src/cli_provider/resume.rs`), and the TUI key handling.

## What "context" means here, and what used to leak

Rewinding already truncated the transcript and soft-deleted the messages in the store. Four
things kept living on as if the removed turns still existed:

1. **The CLI bridge's own session.** `claude`/`codex` keep their conversation server-side; Forge
   resumes it next turn and sends only the delta since a recorded high-water mark. After a rewind
   the transcript is often the *same length* again (the rewound prompt is re-sent, the context pack
   re-injected), so the "did it shrink?" check passed and the bridge resumed a session that still
   contained every turn just removed. Every history rewrite now bumps a per-session **epoch**
   (`CheckpointContext::epoch`); a recorded bridge session from another epoch is never resumed,
   and a parked persistent `claude` process from another epoch is respawned.
2. **"Already injected" latches.** AGENTS.md and the white-hot guidance are injected once and
   latched. If the rewind removed the message that carried them, the latch kept them out for the
   rest of the session. Both latches are now re-derived from the surviving transcript.
3. **Compacted history.** A rewind target inside the part compaction had folded away left the
   model with an empty transcript while the store kept the originals soft-deleted as "compacted".
   The compaction is now undone first, then the rewind applies to the real history.
4. **Turn bookkeeping**: the current-turn seq (so a chained `/undo` lands on the right turn),
   queued diagnostics hints, pending images, and any mid-turn steer prompts are all reset.

`/uncompact` and the "reload full history" resume choice bump the epoch too.

## One seq space

A checkpoint, a snapshot directory and `rewind_to` all speak the DB's `message.seq`. That space
runs ahead of the session's in-memory counter: every side call (recap, suggestion, memory
capture) records its usage by inserting an *inactive anchor row* at `MAX(seq)+1`, so a turn's
user row could land several seqs past the number the session thought it had. Checkpoints were
keyed by the in-memory number and `rewind_to` turned a seq into a transcript index with
`self.seq - transcript.len()`, which only holds when nothing but live rows exist. Rewinding to
turn 3 of 8 then soft-deleted the *tail of turn 2* (its answer and tool results) and left its
prompt active, and across a compaction it emptied the transcript and put turn 1's prompt back in
the input box. Now:

- the turn's user row is inserted first and its **real** seq (`Store::add_message_returning_seq`)
  is what the checkpoint, the snapshot directory and `current_turn_seq` use; a manual
  `/checkpoint` uses `next_seq_for_session`;
- `rewind_to` restores the files of every snapshotted turn `>= seq` (the snapshot directories are
  listed instead of walking a numeric range), deactivates by DB seq and **reloads the transcript
  from the store**, exactly what a fresh resume of that session would see. `self.seq` becomes
  `MAX(seq)+1`: rewound rows keep their seqs, so the next row is never a collision that drifts the
  counter again;
- `/undo` chains through `Store::last_real_user_seq` (the newest active, person-typed user row
  below the current turn), not through transcript offsets.

## The picker

Up/Down move one row; **PageUp/PageDown** move a screenful, **Home/End** jump to the newest and
oldest turn. The row filter keeps a lowercased haystack per row (built once when the list opens),
so a session with thousands of checkpoints filters and scrolls without re-lowercasing the list on
every keystroke and frame (`picker_stays_fast_with_thousands_of_rows` opens 20,000 rows). After a
rewind the cursor is placed at the end of the restored prompt. The app's picker overlay
(`RemoteSnapshot.overlay`) carries a 200-row window around the selection (`body` says "showing
A-B of N"; `selected` indexes the window) rather than every row in every snapshot frame.

## The action menu

Choosing a message opens a second step (`PickerKind::RewindAction`,
`crates/forge-cli/src/cli/commands/run/rewind_menu.rs`) that asks what to do with it. Pick a row
with the arrows and Enter, or press its digit while the focus field is empty:

| # | Row | What it does |
|---|---|---|
| 1 | Code and chat | The old single action: rewind the conversation to before the message and put back the files its turns wrote. Still first, so Enter Enter does what Enter did. |
| 2 | Chat only | Rewind the conversation; the files stay as they are. |
| 3 | Code only | Put the files back and keep the conversation. The model gets a note naming the files that went back (and the message they were put back to), so it reads them again instead of editing from memory. |
| 4 | Summarize | Fold the message and everything after it into one summary and leave the files alone. The folded rows stay in the store, inactive. |
| 5 | Never mind | Back to the message list (Esc does the same). |

Rows 1 and 3 only appear when a turn from that message on left file snapshots to restore
(`Session::has_file_changes_from`); without any, the menu is Chat only, Summarize, Never mind.
Typing in the menu does not filter it: the text is an optional **focus** for the summary, like
`/compact <focus>` (`keep the stack traces`).

All of it goes through `Session::rewind_scoped(seq, RewindScope::{All, Conversation, Code})` and
`Session::summarize_from(seq, focus)` (`crates/forge-core/src/session_rewind.rs`). `rewind_to`,
which `/undo` calls, is `rewind_scoped(seq, All)`. A summary reuses the compaction pipeline
(`Session::fold_summary`, the same budget, size directive and verbatim block `/compact` uses) over
`Store::load_active_messages_from(seq)`, then deactivates those rows, reloads the transcript from
the store and appends the summary as a note headed `[The conversation from this point on was
summarized to save context]`. It runs as a background task with the spinner, and the message goes
back into the input box. A target inside history an earlier compaction folded away reactivates it
first, as for any rewind.

## Keys

- **Esc Esc** (idle): open the rewind picker. A single Esc arms it and the statusline says
  "esc again to rewind". Esc mid-turn still interrupts the response.
- **Ctrl-C**: clear the draft when idle (saved to history), or on an empty input press twice to quit; close an overlay; interrupt mid-turn. Esc no
  longer quits — that was the accidental one-tap exit.

## Tests

`forge-core/src/tests/rewind_menu.rs`: chat only keeps the files, code only keeps the chat and
tells the model, a code restore with nothing to put back adds no note, summarize folds the tail
and keeps the head, summarize past the end does nothing. `forge-cli` (`run/rewind_menu.rs`): the
rows round-trip their action and seq, and the code rows are hidden without snapshots.
`forge-tui` (`picker.rs`): typing does not filter the menu and survives close, a digit picks a row.
`forge-core/src/tests/steer_rewind.rs`: epoch/turn-seq/latch re-derivation, compaction-aware
rewind. `forge-core/src/tests/rewind_seq_space.rs`: checkpoints sit on their user rows with side-call
anchors in the way, rewind/undo deactivate exactly the right rows, rewind across a compaction
restores whole turns, `/new` and `/resume` reset the gauge. `forge-provider` (`cli_provider.rs`): a rewound epoch forbids resuming the CLI session
while the pre-existing guards still hold. `forge-tui` (`app/steer_esc_tests.rs`): the double-tap,
the Ctrl-C/Esc split, the statusline hint.
