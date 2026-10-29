# Feature: steering a running turn from the chat queue

> Type while Forge is busy and the prompt is queued — and now handed to the model at the turn's
> next boundary instead of after the whole turn finishes. Same behaviour as Claude Code and Codex.
> Touches `forge-core` (`steer.rs`, `model_loop.rs`), `forge-types` (`PresenterEvent::Steered`),
> and both chat drivers in `forge-cli`.

## Behaviour

- A prompt submitted while a turn runs is queued **and** pushed into the session's steer inbox.
- The model loop drains the inbox wherever a new user message is legal: right after a tool step's
  results are in, and where a response would otherwise have ended the turn. Each text becomes a
  persisted user message; the surface gets `PresenterEvent::Steered` and echoes it as
  `you · follow-up` where it landed, dropping it from the pending list.
- A prompt still queued when the turn ends (the model answered with no further boundary) starts
  the next turn exactly as before. The inbox is cleared at every turn start so nothing is ever
  delivered twice.
- Slash commands still wait for an idle session.

The session is locked by the turn task for the whole turn, which is why the inbox is a shared
handle (`Session::steer_handle`) the driver takes once at startup rather than a method call.
Bridges see steers naturally: the resume delta carries new user messages.

## The queue is visible, editable, and sendable now

A message that leaves the composer would otherwise vanish (the only trace used to be a `1 queued`
chip on the border and a `⏳ queued (1 pending)` row in the transcript that stayed there after
the message was delivered). The terminal now draws the queue in the live region, directly above
the composer:

```
 2 queued · ↑ edit · Esc stop and send now
   ❯ What is 17 times 23? Reply with only the number.
   ❯ and then the changelog
```

- One dimmed line per message, cut to the width with an ellipsis; at most the newest three are
  listed and the header counts them all. The band is gone the moment the turn takes a message,
  so nothing stale is left in the transcript. Where there is no band to draw (a headless daemon
  mirrored to a phone or `forge attach`) the `⏳ queued` note is still printed.
- **Up**, with the composer empty, pulls the newest queued message back into the composer (its
  pasted images come with it) and withdraws it from the steer inbox so the running turn cannot
  still take it. With text in the composer, Up keeps recalling prompt history.
- **Esc** stops the turn and sends the first queued message now: it always did (the interrupt
  hands the head of the queue to the next turn), the band now says so. It stays out of the way
  while a permission prompt or question owns the keyboard, where Esc means deny or dismiss.

## Queue or steer: `[chat] follow_up`

`follow_up = "steer"` (the default) is the behaviour above. `follow_up = "queue"` makes Enter typed
mid-turn only queue the prompt: it runs after the turn and is never offered to it, which keeps a
long turn on its original task. It is also the `Enter while busy` setting in `/config` and in the
app's Settings. The app's composer sends `RemoteInput::Queue` for it and inverts it on
Cmd/Ctrl+Enter, so one chord does the other thing.

## Editing the queue from the app

The app draws the same queue above its composer (`QueueStrip`): every row can be pulled back to
the composer, moved earlier or later, sent now, or removed. The moves are the
`queue_move { index, text, to }` and `queue_send_now { index, text }` inputs. Like `dequeue`
they echo the row they were rendered from, so an edit made against a queue that has since shifted is
refused with a note. A move reorders the steer inbox with the queue, so the running turn still
absorbs prompts in the order shown; send-now moves the row to the head and interrupts the turn,
exactly what Esc does in the terminal.

## Esc after an absorbed steer

An absorbed steer is already persisted, in the model transcript and echoed. If Esc kills the turn
before it answers, the next turn is started with `Session::run_turn_resuming`: it routes on the
absorbed text but does not persist, push or echo it again (`SteerTracker::interrupted` returns
`InterruptNext::Resume`). A prompt that never reached the model is still an ordinary turn.

## Tests

`forge-core/src/tests/steer_rewind.rs`: injected after a tool step, injected instead of ending the
turn, a leftover from a previous turn is not replayed. `forge-tui/src/app/steer_esc_tests.rs`: the
echo line. `forge-tui/src/app/tests/queue_band.rs`: the band's rows, keys, truncation and its
disappearance. `forge-cli/src/cli/commands/run/queue_images.rs`: pulling a message back out of the
queue and the steer inbox, with and without images.
