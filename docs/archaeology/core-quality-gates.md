# Code archaeology: Core quality gates

Autofix and critic review are post-turn quality mechanisms, not tool policy.
They use snapshots to derive only files changed by the current turn, so they
must not inspect pre-existing worktree state. The Assay gate has a deterministic
cost preflight, reuses live health/catalog candidate selection, and either warns
or emits all qualifying findings before blocking. Autofix failures are injected
as a durable synthetic user turn to preserve a repair trace and resume behavior.

The methods form one deep Session quality boundary; their focused Core tests
cover disabled/no-diff/cost/threshold gates, discovery, pass/fail injection,
and iteration caps.
