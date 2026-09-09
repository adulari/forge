# Feature: stalled tasks — a task nobody can resolve stops driving the session

> Status: **SHIPPED** (2026-09-09). Code: `crates/forge-core/src/task_staleness.rs`, the
> escalating nudge in `crates/forge-core/src/nudge_policy.rs`, wired at turn start in
> `crates/forge-core/src/lib.rs` and on compaction in `crates/forge-core/src/compaction_policy.rs`.

## 1. Problem

Forge's completion authority is the task list: a turn that ends with an unfinished task is a
premature stall, so the harness re-drives the model instead of accepting it. That is correct right
up until the task itself becomes unresolvable.

Observed live (2026-09-09, session `07ca114e`, six days old, ~1.7 B input tokens): an auto
compaction folded away the conversation that produced the task **"Strip live-reuse, keep perf
wins"**, leaving the title, the `In progress` status, and no way to tell which hunks it meant. From
then on:

- every turn ended with one unfinished task, so the completion gate re-drove the model;
- every re-drive spent its steps re-reading the same files to work out what the task meant;
- those reads are tool calls, and `nudge_policy::decide` scores tool calls as progress — so the
  nudge budget refilled instead of running out;
- the session narrated a near-identical sentence each step and made no change to the repository
  until a human interrupted it, decided what the task meant, and marked it done by hand.

None of the existing guards could see this. The doom-loop guard needs identical tool arguments; the
failure-loop guard needs failures; the narration-stall guard (`stall_guard.rs`) bounds the *steps
inside one turn*, not the task that keeps starting new ones. What was missing is the obvious
structural fact: **that task had not moved in days**.

## 2. What ships

`task_staleness::Tracker` counts, per unfinished task, how many consecutive turns its exact
`(title, status)` pair has survived. A task that is renamed, re-statused, finished, or removed
resets — only a genuinely untouched one accumulates. Two escalations follow:

1. **Three unchanged turns → force a decision.** The turn opens with one system line naming the
   task(s) and the four ways out: carry it out with a concrete tool call; mark it Done via
   `update_tasks` saying in one line what was done; remove it via `update_tasks` because it is
   moot; or `ask_user` when only the user can say what it meant. The line says explicitly that
   investigating what the task means is not progress on it — that is the loop.
2. **Two further unchanged turns → Forge drops it.** The task is removed from the list, the model
   is told it was removed and not to re-add it, and the **user** gets a warning naming it, because
   the harness just edited their task list. With the list clear, the completion gate stops
   re-driving and the turn can end normally.

Dropping deliberately does not mean "mark it Done". Done is a completion claim the harness has no
evidence for, and it would send the turn into the verification gate instead of releasing it.

**Compaction awareness.** `Session::compact` tells the tracker that every currently-open task just
lost the context explaining it. If such a task later stalls, the escalation adds: the conversation
was compacted while it was open, so if you cannot reconstruct what it meant, ask the user or drop
it — do not guess. (A compaction that happened *before* a task existed is not blamed for it.)

**Escalating continue-nudge.** Inside one turn the first continue-nudge is unchanged. From the
second on it names the tasks that are still open (capped at 6) and demands the same decision,
because by then the generic instruction has demonstrably not worked — the model answered it and
the work is still open.

## 3. Numbers

| | before | after |
|---|---|---|
| turns a dead task can re-drive the session | unbounded | 5 |
| provider calls spent on it | unbounded (observed: days) | ≤ 5 turn-openings + the turns' own work |
| user told their list contains something unresolvable | never | on the escalation and again on the drop |

## 4. Behaviour at the seams

- **Resume / daemon restart.** The counter is in-memory, so a restart gives a stalled task a fresh
  three turns. The task list itself is persisted, so the count restarts rather than being lost
  permanently.
- **Subagent-assigned tasks.** `assignee` is not part of the key: a task delegated to a subagent
  that never reports still stalls, which is the point.
- **Duplicate titles.** Keyed by title, so two identical titles share one counter; the second is
  treated as a fresh entry each turn and never escalates on its own.
- **Cost.** Pure in-memory bookkeeping over the existing task list — no syscall, no model call.

## 5. Tests

- `task_staleness::tests` — a task being worked on is never touched; a still one escalates exactly
  once then drops; the escalation is not repeated every turn; a compaction while open is reported,
  one before it is not; Done/removed tasks are forgotten; several stalled tasks are named together.
- `tests::stale_tasks_tests` (`crates/forge-core/src/tests/stale_tasks.rs`) — real turns against a
  provider that opens a task and then only talks about it: escalation lands on the third unchanged
  turn without touching the list, the drop lands two turns later and empties the list, and the user
  gets the warning. A task whose status moves each turn is left alone.
- `nudge_policy::tests` — the first nudge is the plain instruction; a repeat names the open tasks
  and the ways out; a long list is capped rather than pasted back whole.
