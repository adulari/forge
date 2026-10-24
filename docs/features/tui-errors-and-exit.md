# Feature: errors, interrupts and exit in the chat TUI

> Three places where the chat used to leave a person guessing: a model id nothing serves, Esc on a
> turn that had already finished, and quitting with no way back. Touches `forge-provider`
> (`error_policy.rs`), `forge-core` (`failure_verdict.rs`, `model_request.rs`), `forge-tui`
> (`chrome.rs`, `notice.rs`) and `forge-cli` (`run/pin_check.rs`, `run/ui_events.rs`,
> `run/exit_hint.rs`).

## A model id nothing serves

`forge chat --model nonexistent-model-xyz` used to show nothing at launch, wait 5 to 6 s, then stack
three messages (a "rejected twice" warning naming `pin_failover`, an unrelated 32k-context note, and
`provider unavailable: 'nonexistent-model-xyz (adapter: OpenAI)'`) under a border that said to send
the message again, which would fail the same way.

**At launch** (`pin_check::check_pins`) every pinned id is checked and one line is printed per id
nothing lists:

```
unknown model 'groq::llama-3.3-70b-versatil' — did you mean groq::llama-3.3-70b-versatile? Run /model to pick another.
unknown model 'nonexistent-model-xyz' — no model like it in the catalog. Run /model to pick another.
unknown model 'nonexistent-model-xyz' — no provider knows it. Run /model to pick another.
unknown model 'nonexistent-model-xyz' — no provider knows it, and Ollama isn't running. Run /model to pick another.
```

- With a catalog the id is judged against it. Suggestions come from the id's own provider when it
  names one; a bare id is matched by name only, and only when the name is genuinely close
  (`close_matches`). It never offers the first few models of the catalog. A script (`forge run`)
  is pointed at `forge models` instead of `/model`.
- With no catalog yet (a first launch) a bare id the provider layer cannot place is dispatched to the
  local Ollama server (`forge_provider::falls_back_to_local_ollama`). A refused connection to it
  answers instantly, so when nothing is listening the warning is printed at launch
  (`no provider knows it`). Ollama is named (`..., and Ollama isn't running`) only when the user
  has set it up: `[local] model`, `[local] autostart` or `OLLAMA_HOST`. When Ollama is running the
  id may be a pulled model and nothing is claimed.
- The ids flagged here are the session's `unverified_models`. `/model <id>` marks an id the catalog
  does not list the same way.
- A bare id that only the absent Ollama would have taken is also the session's
  `unavailable_models` (`pin_check::PinCheck`). While the whole pin is in that list the composer
  does not send: Enter keeps the text in the composer and prints one line,
  `not sent: model 'x' isn't available. Run /model to pick another.` Pinning another model with
  `/model` lifts it. An id that is merely missing from the catalog still sends (a brand-new model
  may not be discovered yet).

**When the turn fails** the answer is immediate and one line:

```
✗ turn failed: model 'nonexistent-model-xyz' isn't available (no provider knows this id). Run /model to pick another.
```

The classification, not a timeout, is what removed the wait: a provider's "no such model" answer is
`ProviderError::NoModelAccess` (permanent), a bare id whose request found no Ollama is classed the
same way, and a pinned permanent error is `FailTurn` on the first call (see mesh-routing.md, "A pin
nothing serves"). The provider's own text goes to `.forge/forge.log`. The composer border no longer
states it a third time: its hints lead with `/model pick another` and nothing else; no `Next:` line
repeats it, and the "context window is unknown" note is not stacked on a pin
startup already flagged.

## Esc

Esc during a running turn stops it and prints `Interrupted. Type what to do instead.` (it used to
say `interrupted — stopped responding`, which read like a hang).

Esc on a turn that has just finished is not an interruption. The loop clears its `busy` flag from
the turn's done signal, one pass after the turn's last events are queued, and a task that is past
its last await keeps running to its result event even after an abort. An Esc in that gap used to
abort the task, discard the queued events (the answer's closing lines and its usage row) and print
the notice under a complete answer; on a mock turn Esc sent the moment the last line rendered did
this 12 times in 12. Now the loop

1. applies what the turn has queued (`ui_events::apply_ui_msgs`) and leaves the turn alone when its
   result is on screen or its task has exited (`esc_has_nothing_to_stop`);
2. otherwise aborts the task and waits up to 30 ms for it to stop (`stop_turn_task`; not while a
   permission prompt or question holds the task, which only the dropped answer channel can wake),
   applies what it emitted before it stopped, and leaves the turn alone if that included the
   result event;
3. only then treats the Esc as an interrupt.

When the Esc is left alone the rest of that batch of input waits one pass, until the done signal
has cleared `busy`; keys typed behind the Esc (`/quit`) would otherwise meet a turn that is already
over and be refused as "can't run mid-turn".

A `/loop` or `/goal` is the exception to every "leave it alone": Esc also stops the loop that would
start the next turn. An Esc that lands before the result event is a real interrupt even when the
answer's last line is already on screen; it prints the same plain notice.

## Exit

Leaving the chat prints how to come back, after the terminal is restored so it lands in the
scrollback and not on the alternate screen:

```
Resume this session: forge chat --resume 34a89baa  "Fix the failing test"
```

The id is the eight characters the rest of the interface shows; the session's name follows when it
has one, cut to fit the terminal width (the command is never cut). A session with no messages prints
nothing. `forge chat --plain` prints the same line when stdout is a terminal.

## Tests

- `forge-provider`: `genai_provider/error_policy/tests.rs` (each provider's unknown-model answer is
  permanent, the refused connection keeps its cause, the bare-id fallback).
- `forge-core`: `failure_verdict.rs`, `a_pinned_model_nobody_serves_fails_at_once_in_one_plain_line`,
  `tests/startup_overlap.rs` (the window notice).
- `forge-tui`: `app/tests/chrome_surfaces.rs` (border and hints), `app/tests/transcript.rs` (no
  `Next:` line).
- `forge-cli`: `run/pin_check.rs` (suggestions, launch warnings), `run/exit_hint.rs`,
  `run/ui_events.rs`, and the pty tests `tui_esc_*`, `tui_quit_prints_how_to_resume_*` in
  `tests/tui_e2e.rs` (`--ignored`).
