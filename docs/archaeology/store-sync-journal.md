# Code archaeology: Store Anywhere sync journal

Anywhere journal rows are encrypted only after Store has created a durable,
canonical plaintext snapshot. The associated upload envelope must be sealed
exactly once so retrying workers reuse authoritative bytes. Download cursor
advancement must be monotonic and a staged remote revision must be verified and
persisted in the same transaction as its cursor; otherwise crashes can skip or
replay unverified remote data.

This is a cohesive persistence boundary distinct from higher-level memory,
history, and file conflict application. The extraction retains all SQL and
transaction scopes in `Store`, moving only the journal/staging owner.
