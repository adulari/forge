# Core session context program split

This high-value Core phase replaces the 1,316-line mixed context block with
three private deep owners:

- `routing_policy.rs` (465 lines): readiness/budget snapshots, route inspection,
  cache-stable preambles, bounded request assembly, planner/editor selection,
  and auxiliary model selection;
- `compaction_policy.rs` (418 lines): token accounting, admission, automatic
  compaction, failover, health accounting, and persistent compaction/undo;
- `auxiliary_policy.rs` (464 lines): bounded recap, durable memory capture,
  suggestion, and shell-diagnosis side calls.

They remain `impl Session` methods, preserving state locality and call order.
No public session surface, request sequence, event ordering, Store transaction,
context compaction semantics, provider state, failover order, affinity, or
permission behavior changes.

## Archaeology-backed invariants

Context must be recalculated for each active/failover model and use one bounded
provider view; summaries persist as soft-deleted source messages plus a visible
marker; context-overflow self-healing only shrinks its affected model window;
optional side calls never delay or invalidate a primary completion; model health
scope distinguishes model capability from provider authentication failure.

## Results

Core root implementation lines reduce from 10,332 to 9,019, removing the final
>10,000 LOC implementation file. All new owners are below 500 LOC. The
architecture guard reports no implementation file above 10,000 LOC.

## Verification

Warnings-denied Core Clippy, all 507 Core library tests, all three long-session
endurance tests, formatting, and the architecture guard passed. The regression
set includes compaction/resume/undo, overflow retry, context fitting, route
explanation/affinity, candidate failover, provider health, auxiliary bounds,
replay, and repeated long-session recovery.
