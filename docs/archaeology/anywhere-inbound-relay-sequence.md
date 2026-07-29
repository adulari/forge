# Code archaeology: Anywhere inbound relay sequence acceptance

## Boundary

`connector/inbound_sequence.rs` owns durable replay acceptance after the parent connector has authenticated and decrypted a relay envelope (and, when present, its referenced blob). The parent remains responsible for cryptographic validation and local dispatch.

## Invariants

- Sequence numbers are scoped by sender device and account key epoch.
- A referenced blob must have a lower sequence than its control envelope.
- Blob and control acceptance are one `StateStore::update` transaction: a rejected pair commits neither sequence.
- A control envelope cannot replay or move behind the stored watermark, including after state reload.

## Characterization

`blob_and_control_sequences_are_accepted_atomically` proves advancing pairs persist, an invalid blob/control ordering rolls back, the same blob can subsequently be accepted with a fresh control sequence, and reloaded state rejects replay.
