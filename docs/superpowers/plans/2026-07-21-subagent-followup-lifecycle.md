# Subagent Follow-up Lifecycle Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ensure a completed follow-up to a persisted subagent finalizes the active follow-up row instead of leaving it permanently running.

**Architecture:** Keep the existing append-only activity history: a follow-up start may add a second row with the same persisted child ID. Reconcile progress and results against the active unfinished row, preserving earlier completed rows for transcript history.

**Tech Stack:** Rust, `forge-agent-tui`, presenter event state, Cargo tests.

## Global Constraints

- Preserve completed rows from earlier batches in the same parent turn.
- A `SubagentResult` for a reused child ID must update the active `done == false` row.
- Do not change child-session persistence, address resolution, or create a second child session.
- Keep the issue open until a real spawn → result → `send_to_agent` → result flow leaves no running row in the remote snapshot.
- Do not modify unrelated files.

---

### Task 1: Reconcile a Follow-up Result to the Active Row

**Files:**
- Modify: `crates/forge-tui/src/app.rs:1975-2018`
- Test: `crates/forge-tui/src/app.rs` test module near `subagent_panel_present_while_running_and_after_done_collapses_on_next_turn`

**Interfaces:**
- Consumes: `PresenterEvent::SubagentStart`, `PresenterEvent::SubagentResult`, and `App::running_subagents()`.
- Produces: result reconciliation that selects `SubRow { id, done: false, .. }` for reused IDs.

- [ ] **Step 1: Write the failing regression**

Add a test that applies `SubagentStart(id = "persisted-child")`, its first successful `SubagentResult`, a second `SubagentStart` with the same ID and a follow-up task, then the second result. Assert two historical rows exist, the second row is running before its result, and after the result both rows are done and `running_subagents() == 0`.

- [ ] **Step 2: Run the regression and verify RED**

Run: `cargo test -p forge-agent-tui follow_up_result_finishes_the_active_reused_id_row --lib`

Expected: FAIL because the result lookup selects the first already-completed row, leaving the follow-up row with `done == false` and `running_subagents() == 1`.

- [ ] **Step 3: Implement the minimal reconciliation fix**

Change the `PresenterEvent::SubagentResult` row lookup from the first matching ID to the matching active row:

```rust
if let Some(row) = self.subagents.iter_mut().find(|r| r.id == id && !r.done) {
```

Do not change start-row retention or child ID reuse.

- [ ] **Step 4: Verify GREEN and the focused package gate**

Run: `cargo test -p forge-agent-tui follow_up_result_finishes_the_active_reused_id_row --lib`

Expected: PASS.

Run: `cargo test -p forge-agent-tui subagent --lib`

Expected: all matching subagent tests PASS.

Run: `cargo fmt --check && cargo clippy -p forge-agent-tui --all-targets -- -D warnings`

Expected: formatting and clippy PASS with no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/forge-tui/src/app.rs docs/superpowers/plans/2026-07-21-subagent-followup-lifecycle.md
git commit -m "fix(tui): finish active subagent follow-up rows"
```

---

### Task 2: Live Lifecycle Verification

**Files:**
- No production source changes expected.
- Record durable evidence in the pull request and issue only after the live check passes.

**Interfaces:**
- Consumes: a built Forge host containing Task 1 and an actual session with subagents enabled.
- Produces: remote snapshot evidence that the reused child has no unfinished follow-up row.

- [ ] **Step 1: Build and run the corrected Forge host**

Run the normal production-equivalent Forge binary build and launch path for this machine.

- [ ] **Step 2: Exercise the real lifecycle**

In one real Forge parent session: spawn one named child, wait for its first result, call `send_to_agent` for that same child, and wait for the follow-up result.

- [ ] **Step 3: Inspect the remote snapshot**

Verify the snapshot contains the historical first row and follow-up row, both `done: true`, while the parent is idle. Confirm clients report zero running subagents without a refresh or extra prompt.

- [ ] **Step 4: Close only after evidence passes**

If and only if the live flow passes, add the evidence to #849 and close it. Otherwise keep #849 open with the observed failure.
