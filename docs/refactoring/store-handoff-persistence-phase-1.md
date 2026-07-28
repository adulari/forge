# Store internal split — handoff persistence phase

This is a mechanical Store extraction under ADR-0005 and the canonical
architecture campaign. It moves Forge Anywhere handoff ownership out of the
Store root without changing the public Store seam or database contract.

## Ownership

- `handoff_types.rs`: stable encrypted-capsule data records, deliberately
  re-exported from `forge_store` for compatibility.
- `handoff.rs`: export/import, provenance, source/destination state transitions,
  quarantine, activation, and rollback.
- `lib.rs`: Store construction, shared connection/transaction helpers, and
  deliberate public re-exports.

The lifecycle implementation is 408 lines and the data owner is 64 lines; both
are below the 500-line implementation target. The root drops from 8,298 to
7,852 implementation lines. This is a deep extraction because a deleted
handoff owner would force its state machine, capsule records, and transaction
protocol back into the general Store root.

## Non-change contract

No schema, SQL statement, transaction boundary, Store public method path,
serialization shape, Anywhere protocol, session archive behavior, or sync
snapshot behavior changes. `Store` still exclusively encapsulates SQLite.

## Verification

- `cargo fmt --all -- --check`
- warnings-denied Store Clippy across targets/features
- Store library tests: 127 passed
- `python3 scripts/ci/architecture_size.py`: passed

The focused tests include capsule import collision/remap/rollback, destination
quarantine/activation, source cancellation, transferred-source immutability,
and persistence concurrency/migration checks. No model-visible behavior,
runtime provider operation, dependency, or schema changed; a paid benchmark is
not applicable.
