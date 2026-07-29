# Core quality gates split

This phase isolates post-turn quality gates in private `quality_gates.rs`:
snapshot-derived diff construction, Assay critic selection/cost gate/findings
policy, zero-config autofix discovery, and synthetic failure feedback injection.

The extraction keeps the sequential invariant: snapshot-derived diffs are
reviewed only after a turn's writes; an enabled autofix failure persists a
synthetic user message before the next repair iteration; Assay block mode emits
all findings before it aborts the turn. No model selection, write permission,
Store transaction, snapshot, or completion behavior changes.

Core root implementation lines reduce from 6,363 to 6,093; this owner is 283
lines. Core Clippy, 507 Core tests, 3 long-session endurance tests, formatting,
and the architecture guard passed.
