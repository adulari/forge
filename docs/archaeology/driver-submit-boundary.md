# Code archaeology: remote input submission

## Boundary

`run/driver/submit.rs` owns what a remote client's *input* does to a daemon-hosted session:
handling each `RemoteInput` variant and the idle-state submit routing (`//` escape, `/command`
dispatch, plain prompt). It sits beside `driver/input.rs`, which owns headless *key* routing; the
driver keeps the session loop, turn lifecycle, and snapshot publication.

## Two behaviours decided at input time

Both live here rather than in the turn machinery because they are settled before a turn exists:

- Input that arrives while a turn is running is **queued**, not dropped — a phone that sends two
  messages in a row must not lose the second.
- A `Prompt`'s own message-correlated attachment list is **authoritative for that turn** (the
  mobile upload race), discarding ambient state left by an unrelated earlier `Attach`. When the
  list is absent, the old ambient `pending_mentions` behaviour is preserved exactly.

## Interface

`handle_input` and `submit_line` stay inherent methods on `DriverState`, promoted to `pub(super)`;
the driver loop's call sites are unchanged.

## Characterization

The driver and `serve` test suites (81 tests) pass unchanged, including the remote-input,
attachment-correlation, and queued-while-busy behaviours.
