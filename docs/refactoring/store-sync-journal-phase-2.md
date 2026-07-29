# Store sync journal split

This mechanical Store phase isolates the encrypted Anywhere journal, upload
envelope, cursor progression, and verified remote-record staging into private
`sync_journal.rs`. The implementation remains on `Store`; no database access,
transaction boundary, sync protocol, or public Store API changes.

The moved owner preserves the critical coupling of record validation, revision
conversion, idempotent insert/lookup, and cursor advancement in IMMEDIATE
transactions. Its tests cover duplicate envelopes, cursor monotonicity,
verified staging conflicts, and concurrent Store writers.

Store root implementation lines reduce from 7,852 to 7,564; the new owner is
296 lines. Warnings-denied Store Clippy, all 127 Store tests, formatting, and
the architecture guard passed.
