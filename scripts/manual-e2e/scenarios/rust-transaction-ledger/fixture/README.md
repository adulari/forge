# Transaction Ledger

Repair the in-memory transfer ledger without changing its public API.

`Ledger::new` must preserve account creation order and reject empty account IDs and duplicates.
`Ledger::apply_batch` applies transfers sequentially, so a later transfer may spend funds received
earlier in that same batch. The whole batch is atomic: any validation or arithmetic failure leaves
all balances and idempotency state unchanged.

Rules:

- Batch IDs must contain at least one non-whitespace character.
- Transfer amounts must be greater than zero and source/destination accounts must differ.
- Both accounts must exist. Errors identify the zero-based transfer index.
- Insufficient funds and destination `u64` overflow are errors; never saturate or wrap.
- Retrying the same batch ID with byte-for-byte-equivalent transfers returns the original receipt
  and does not apply anything again, even after other batches have changed the ledger.
- Reusing a batch ID for different transfers returns `BatchConflict` without mutation.
- Receipts and snapshots list accounts in original creation order.
- Empty batches are valid and idempotent.

Use only the standard library. Do not weaken tests or change exported names, fields, variants, or
function signatures. Acceptance requires formatting, Clippy with warnings denied, and all tests.
