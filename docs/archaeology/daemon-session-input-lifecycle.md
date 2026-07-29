# Code archaeology: daemon session input lifecycle

## Summary

The daemon session driver has two independent responsibilities: owning the long-lived session task and translating remote input into the same modal actions and turn lifecycle used by the TUI. The latter is now isolated as the headless input lifecycle boundary. Event precedence, generation checks, FIFO prompt draining, and overlay completion semantics are unchanged.

## History and invariants

- `e3708644` introduced the daemon-hosted driver and established modal-first remote key routing without a terminal.
- `c908811c` preserved queued prompts across interruption and used turn generations so a late completion signal cannot stop replacement work.
- `3e6e552d`, `74298fbd`, and `94b26dc2` added autonomous goal continuation, user-facing completion, and bounded automatic compaction.
- `a3d90e33` correlated uploaded attachments with their own prompt rather than ambient queue state.
- `18f66e70` added removal of one queued remote prompt without disturbing FIFO order.
- `526fc672` added stress characterization for long prompt queues and stale completion signals.

## Boundary

`driver/input.rs` owns:

- modal remote-key precedence and headless-only restrictions;
- turn-completion generation checks and autonomous continuation;
- FIFO draining after completion or interruption;
- asynchronous mesh/usage overlay completion and dirty-frame signaling.

The parent driver retains task creation, input ingestion, snapshot publication, push notification ordering, and shutdown. Only the three lifecycle entry points called by the parent loop are visible to the parent module.

## Interface as test surface

The existing driver tests characterize the extracted lifecycle directly:

- `interrupt_with_queue_starts_fifo_head_and_keeps_driver_busy`;
- `stale_interrupt_done_signal_cannot_stop_fifo_drain`;
- `queued_reprompt_steers_the_next_loop_iteration`;
- `queued_reprompt_steers_the_next_goal_iteration`;
- `interrupt_without_queue_leaves_driver_idle`;
- `over_a_thousand_queued_reprompts_drain_fifo_without_stale_done_corruption`;
- `mesh_overlay_resolution_reports_a_dirty_frame`.

These tests exercise the new owner through `DriverState`, making deletion or ordering drift observable.

## Leave alone

- Modal surfaces must consume keys before plain input.
- A completion signal may act only on its matching generation.
- Interrupting active work must not discard queued prompts.
- Queue draining must remain FIFO and autonomous loop/goal steering must consume the next queued prompt first.
- Overlay resolution must mark the next frame dirty; otherwise remote clients remain on stale loading state.
- Headless `/quit` remains parent-driver policy: it is session-scoped and must never terminate the daemon hosting other sessions.
