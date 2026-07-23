# Transaction ledger reference

The fixture starts with six failing tests across account validation, atomic rollback, checked
overflow, exact idempotent retries, and conflicting batch IDs. The reference acceptance commands are:

```bash
cd reference
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
```

## Verified unpinned-mesh result (2026-07-23)

A fresh live TUI run exercised four free API models while recovering across NVIDIA Minimax,
NVIDIA DeepSeek V4 Pro, Thinking Machines Inkling, and Mistral Devstral. Forge surfaced and
recovered one empty response plus an A→B→A tool-loop warning. The model rejected its own first
non-atomic fix, replaced in-place mutation with a working-copy commit, and then passed formatting,
strict Clippy, and all 8 tests; the scenario runner independently repeated the checks. All 33
persisted tool envelopes/execution records were valid. Two non-OK tool outcomes were expected,
observed checks that the turn subsequently corrected. The full workspace and TUI timeline remain
under `scripts/.manual-e2e-out/rust-transaction-ledger-20260723T031211Z-1860574/`.
