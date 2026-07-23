# Reservation Service Repair

This repository contains an in-memory asynchronous inventory reservation service. The public API is
small, but the current implementation violates several production invariants.

Repair the implementation without changing the public method signatures or weakening tests.

Required behavior:

- Reservation quantities must be positive and SKUs must exist.
- A request ID is idempotent: repeating the same request returns the original reservation and
  decrements inventory only once.
- Reusing a request ID with different SKU or quantity is a conflict.
- Inventory checks and decrements are atomic across concurrent coroutines.
- A storage failure must leave inventory and reservations unchanged.
- Cancellation is idempotent and restores inventory exactly once, including under concurrency.
- Active reservations are returned in creation order and cancelled reservations are excluded.
- Errors use the exception types exported by `reservations.models`.

Run the complete suite with `python -m unittest discover -v`.
