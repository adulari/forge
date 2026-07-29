# Code archaeology: queue drain support

## Boundary

`cli/commands/queue/support.rs` owns the autopilot drain's pure support surface: the
`forge run --output-format stream-json` fold, branch slug derivation, display truncation and
status glyphs, workspace resolution (canonical cwd, git repo root), and the best-effort desktop
notification. Queue command dispatch, store mutation, worktree lifecycle, budget/gate policy, and
the scoreboard remain in the queue command owner.

## Interface

`StreamRun` plus the free helpers are `pub(super)`: they exist for the queue command owner and its
tests, not as a crate-wide utility surface. The stream fold remains the contract with the child
run's NDJSON events (`init` → session id, `routing` → routed model, `usage` → cumulative cost,
`result` → final text), where the last `routing` event wins so mid-run failover reports the model
that actually finished the work.

## Characterization

The parent module's tests cover stream folding across malformed lines, slug safety and budget,
and char-boundary-safe truncation. They continue to exercise the extracted helpers through the
parent's imports, so the extraction is behavior-preserving rather than a line-count move.
