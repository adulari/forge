# Transaction ledger reference

The fixture starts with six failing tests across account validation, atomic rollback, checked
overflow, exact idempotent retries, and conflicting batch IDs. The reference acceptance commands are:

```bash
cd reference
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
```
