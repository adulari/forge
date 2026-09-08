# Feature: steering a running turn from the chat queue

> Type while Forge is busy and the prompt is queued — and now handed to the model at the turn's
> next boundary instead of after the whole turn finishes. Same behaviour as Claude Code and Codex.
> Touches `forge-core` (`steer.rs`, `model_loop.rs`), `forge-types` (`PresenterEvent::Steered`),
> and both chat drivers in `forge-cli`.

## Behaviour

- A prompt submitted while a turn runs is queued (as before, shown as "⏳ N queued") **and**
  pushed into the session's steer inbox.
- The model loop drains the inbox wherever a new user message is legal: right after a tool step's
  results are in, and where a response would otherwise have ended the turn. Each text becomes a
  persisted user message; the surface gets `PresenterEvent::Steered` and echoes it as
  `you ⚡ steer` where it landed, dropping it from the pending list.
- A prompt still queued when the turn ends (the model answered with no further boundary) starts
  the next turn exactly as before. The inbox is cleared at every turn start so nothing is ever
  delivered twice.
- Slash commands still wait for an idle session.

The session is locked by the turn task for the whole turn, which is why the inbox is a shared
handle (`Session::steer_handle`) the driver takes once at startup rather than a method call.
Bridges see steers naturally: the resume delta carries new user messages.

## Tests

`forge-core/src/tests/steer_rewind.rs`: injected after a tool step, injected instead of ending the
turn, a leftover from a previous turn is not replayed. `forge-tui/src/app/steer_esc_tests.rs`: the
echo line.
