# Code archaeology: Store handoff persistence

## Summary

Forge Anywhere handoff persists an encrypted-session transfer as a strict local
state machine. Export/import records, capsule provenance, source freeze,
destination quarantine, acknowledgement activation, transfer finality, and
rollback must retain their existing transaction boundaries. The split keeps
these lifecycle operations together in `handoff.rs` and keeps the capsule data
contract in `handoff_types.rs`; `Store` remains the only SQLite interface.

## Timeline

- **2026-07-19, `70ecdb87` / PR #811:** added encrypted Anywhere handoffs,
  idempotency and replay protections, destination quarantine, and durable
  provenance.
- **2026-07-24, `380155eb` / PR #890:** hardened Store write/open behavior and
  reinforced the need not to combine storage movement with behavioral changes.

## Load-bearing invariants

- The complete destination import, provenance insert, quarantine row, and
  sync snapshot commit atomically.
- A source handoff archives the source before network work; only a cancellation
  of the exact pending capsule may unarchive it.
- Transferred sources are terminal; normal archive controls cannot resurrect
  them.
- A destination remains quarantined until the service acknowledgement, and
  rollback removes the imported session on acknowledgement failure.
- Capsule exports include transcript/checkpoints only: no credentials, indexes,
  caches, schedules, or queue state.

## Evidence

`handoff_session_import_remaps_collisions_and_rolls_back_cleanly`,
`destination_import_stays_quarantined_until_explicit_activation`,
`cancelled_source_handoff_becomes_resumable`, and
`source_handoff_freeze_survives_archive_controls_and_transfer` exercise these
properties through the public Store seam.

## Safe extraction

Move the cohesive lifecycle and compatibility record types without changing SQL,
transaction behavior, public Store method paths, or the capsule format.
