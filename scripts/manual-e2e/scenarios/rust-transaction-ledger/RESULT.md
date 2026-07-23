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
observed checks that the turn subsequently corrected. The full workspace and TUI timeline are
retained as `rust-transaction-ledger-20260723T031211Z-1860574` in Forge's persistent
`manual-e2e-runs/` directory.

## Second verified unpinned-mesh result (2026-07-23)

A second fresh TUI run routed entirely through the free mesh, moving from NVIDIA Minimax to NVIDIA
DeepSeek V4 Pro and using an OpenRouter auxiliary diagnosis. The models produced and rejected two
incorrect implementations: first a compile-time tuple mistake, then a non-sequential validation
strategy that broke valid dependent transfers. Forge surfaced each failing check, recovered an
optional diagnosis timeout, triggered its A→B→A loop guard, and retained buffered-provider
activity throughout. The final snapshot-and-rollback implementation passed formatting, strict
Clippy, all 8 tests, and the runner's independent repetition.

All 21 persisted tool envelopes/executions were structurally valid. The session used 9.9k output
tokens and reported 91.6k cached input tokens without an output cap. The full result is retained as
`rust-transaction-ledger-20260723T040807Z-1940946` in Forge's persistent `manual-e2e-runs/`
directory.
