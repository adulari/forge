# Code archaeology: automation persistence

## Summary

Schedules, queued tasks, and saved-workflow runs are three durable automation lifecycles. They share one persistence module because callers coordinate them as machine-local automation state, while their table-specific transition guards remain explicit. The extraction preserves project scoping, atomic claims, terminal-state guards, crash recovery, and observed-time honesty.

## History and invariants

- `d262f781` introduced schedule registration and enabled/last-run state; operating-system timer installation remains caller-owned.
- `f3b7ae43` introduced the overnight queue with pending-to-running compare-and-set claims so concurrent drains cannot execute one task twice.
- `42225e49` introduced saved-workflow run history scoped by workflow name and workspace, together with stale-running projection and terminal-write guards.

## Boundary

`automation_store.rs` owns schedule registry state, queue lifecycle state, and saved-workflow execution history. Counterfactual session forks remain in the parent store even though queue completion is adjacent: forks own transcript-copy and ancestry semantics, not automation scheduling.

## Interface as test surface

- `schedule_roundtrips_list_last_run_and_remove` covers registry, enablement, tick time, prefix matching, and deletion.
- `queue_task_roundtrips_claim_finish_and_remove` covers project filtering, claim state, finish fields, and removal rules; `a_late_queue_finisher_cannot_overwrite_a_terminal_outcome` covers terminal-state protection.
- `workflow_runs_record_their_outcome_and_stay_scoped_to_one_workspace` covers history isolation and outcomes.
- `an_interrupted_workflow_run_never_keeps_claiming_it_is_running` covers explicit and stale interruption projection.

Deleting the module removes these public `Store` APIs and breaks their direct lifecycle characterizations.

## Leave alone

- Schedule enablement records desired state but does not install or remove an OS timer.
- Queue claims remain compare-and-set from `pending`; running tasks cannot be removed, and late finishers cannot overwrite terminal outcomes.
- Queue project filtering uses the persisted working directory.
- Workflow history reads stay scoped by workflow name and workspace; starting any workflow globally repairs all stale running rows before inserting the new row.
- A known interrupt records its finish time; crash-stale projection does not invent one.
- Terminal workflow outcomes cannot be overwritten by a late interrupt.
