# Code archaeology: assay and checkpoint persistence

## Summary

Assay analysis records and conversation rewind checkpoints are independent persistence domains that were embedded in the store root. They now have dedicated owners while preserving finding ranking, scope history, soft-delete auditability, sync-journal atomicity, and checkpoint ordering.

## History and invariants

- `8ea0b5b6` introduced assay runs and findings; one run owns its findings.
- `3d1eb7a5` added scope-specific latest-run lookup for report diffing.
- `5983ebef` fixed finding reads to rank by severity then confidence; insertion order is not presentation order.
- `a7fd1dfa` introduced checkpoints and rewind as soft-deactivation, preserving audit/redo rows.
- `70ecdb87` made checkpoint creation and its sync-journal revision one immediate transaction.

## Boundaries

`assay_store.rs` owns assay runs, findings, ranked finding reads, latest-run lookup, and run history. `checkpoint_store.rs` owns transcript soft-deactivation, checkpoint creation with sync journaling, and newest-first checkpoint projection.

The store root retains message sequencing because it is shared by all transcript writes, not only rewind. Compaction soft-deletion also remains there: it marks `compacted = 1`, while rewind leaves `compacted = 0`, so uncompact cannot resurrect messages intentionally removed by undo.

Checkpoint creation is sync-journaled atomically. Message deactivation itself remains a local rewind mutation and is not emitted as a separate sync revision; this extraction preserves that existing asymmetry rather than claiming peer-replay atomicity for both operations.

## Interface as test surface

`assay_run_and_findings_round_trip`, `assay_findings_are_ranked_by_severity_then_confidence`, and `assay_history_is_scope_specific_and_excludes_the_current_run` characterize persistence, ranking, scope comparison, and newest-first history. `checkpoints_round_trip_newest_first` characterizes labels, sequence boundaries, and ordering. Core undo, compaction, rewind, and Anywhere sync tests continue to exercise soft-deactivation and checkpoint journal replay through the public `Store` API.

Deleting either module removes the corresponding public `Store` API and breaks these direct characterizations.

## Leave alone

- Findings remain children of one run and are ranked by severity then confidence.
- Latest-run lookup excludes the just-created run and remains scope-specific.
- Rewind is a soft delete; historical rows remain durable.
- Checkpoint creation and its sync-journal write remain one immediate transaction with busy retry; message deactivation is not separately journaled.
- Named checkpoints are returned newest sequence first.
