# Feature: `/rewind` — one command that rewinds chat, context and files together

> `/rewind` (alias `/undo`, or press **Esc twice** while idle) opens the message picker; choosing a
> message rewinds the conversation to just before it, restores the files those turns wrote, and
> puts the message back in the input box. Touches `forge-core` (`session_history.rs`), the CLI
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

## Keys

- **Esc Esc** (idle): open the rewind picker. A single Esc arms it and the statusline says
  "esc again to rewind". Esc mid-turn still interrupts the response.
- **Ctrl-C**: quit when idle with nothing open; close an overlay; interrupt mid-turn. Esc no
  longer quits — that was the accidental one-tap exit.

## Tests

`forge-core/src/tests/steer_rewind.rs`: epoch/turn-seq/latch re-derivation, compaction-aware
rewind. `forge-provider` (`cli_provider.rs`): a rewound epoch forbids resuming the CLI session
while the pre-existing guards still hold. `forge-tui` (`app/steer_esc_tests.rs`): the double-tap,
the Ctrl-C/Esc split, the statusline hint.
