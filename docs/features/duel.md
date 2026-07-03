# Feature: model arena with routing learning (`/duel`)

> **Status: MVP shipped.** `/duel <task>` runs the SAME coding task through 2-3 mesh models
> concurrently, each in its own isolated git worktree. The user picks a winner from a comparable
> picker (diffstat / test badge / duration / cost); the winner's branch merges back and the
> outcome softly biases future routing in this repo.

## 1. Problem (JTBD)
> When I'm not sure which model will do the best job on a task, I want to just race a few of them
> on the real thing and pick the best result — instead of guessing from a single mesh pick, or
> manually re-running the same prompt against different models myself.

## 2. Scope (MoSCoW)
**Must have (shipped)**
- `/duel <task>` (TUI command + palette entry) routes to up to 3 distinct-provider models via
  `Router::route_candidates` and runs the task in each one's own worktree, concurrently.
- Each candidate is a full write-capable child agent (read/write/shell tools), pinned to its
  specific model (`ResolvedAgent.pinned_model` bypasses the mesh entirely — the candidate MUST run
  the model the arena picked, not whatever the mesh would independently route the task to).
- After each candidate finishes: diffstat (`git diff --shortstat HEAD..<branch>`), an auto-detected
  test run in its own worktree (best-effort — `None` when no test command is recognized), wall
  time, and cost.
- A picker shows every candidate (model + ok/fail badge, diffstat, test badge, duration, cost,
  one-line summary). Enter merges the picked branch back (`merge_worktree_back`, the same 3-way
  patch machinery subagent worktree-isolation uses) and records every candidate's outcome. Esc
  discards everything — no merge, no routing-learning record.
- The outcome (`forge_store::duel_outcome`) softly biases future routing **in this repo**:
  `Store::duel_boosts` aggregates wins-minus-losses per model into a bounded boost
  (`HeuristicRouter::with_repo_boosts`), applied as a stable reorder over the ranked candidate list
  so an unrelated tie-break can't be overridden by a single lucky win.

**Deferred**
- Cross-repo / global routing learning (boosts are scoped to one repo's canonicalized cwd).
- Streaming per-candidate diffs into the picker before the duel finishes (only the final report is
  shown).
- More than 3 candidates, or letting the user hand-pick which models race.

## Non-goals
- No change to the single-model turn path — `/duel` is an opt-in, separate flow.
- Not a benchmark harness: it's a one-shot arena for a real task on real repo state, not a
  repeatable eval suite.

## 3. Acceptance criteria
```
Given a git repo and at least 2 usable distinct-provider models
When /duel <task> runs
Then each candidate implements the task in its own worktree concurrently, and a picker shows
 diffstat / test badge / duration / cost per candidate

Given the user picks a winner
When Enter is pressed on a candidate row
Then that candidate's branch is merged back into the main tree, every candidate's outcome is
 recorded (won only for the winner), and every worktree+branch is removed

Given the user cancels (Esc)
When the picker closes
Then nothing is merged, nothing is recorded, and every worktree+branch is removed

Given the repo has duel history
When the mesh next routes a task in this repo
Then a model with more duel wins than losses here ranks above an otherwise-equal peer
```

## 4. Design

**Routing (`forge-mesh`)** — `Router::route_candidates` (default-implemented as a single `route()`
call, so `FixedRouter`-style test doubles still satisfy the trait) is overridden on
`HeuristicRouter`: classify the task once, rank candidates for that tier, then walk the ranked list
taking the first model per distinct provider until `n` are picked. Each becomes a full
`RoutingDecision` (`rationale: "duel candidate #i — <classify reason>"`).
`HeuristicRouter::with_repo_boosts(HashMap<model, f64>)` stores a per-model boost; the SAME boost
map is applied inside `candidates_for_tier` (both the auto-discovery and configured paths) as a
stable sort by boost descending, so it also nudges ordinary single-model routing, not just duels.

**Orchestration (`forge-core::duel`)** — `duel::run(ctx, parent_id, budget, task, on_event)`:
requires a git repo; routes candidates; for each, creates a persisted child session +
`WorktreeGuard` (same isolation `subagent::orchestrate` uses for write-capable children), a
`ResolvedAgent` with `pinned_model: Some(model)` and the full read/write/shell toolset, and a child
`AgentCtx` with `mode: PermissionMode::AcceptEdits` and `worktree_root` pointed at the guard's path.
Children run concurrently via `tokio::spawn` + an `mpsc` channel draining into `Lifecycle`
Start/Progress/Done events (mirrors `subagent::orchestrate`'s shape, reusing its `Lifecycle` type
directly — a duel shows up in the same activity panel as `spawn_agents`). After each child
finishes: `git diff --shortstat` parsed into `(files_changed, added, removed)`; a zero-config test
command detected from the WORKTREE's contents (`Cargo.toml` → `cargo test --workspace`,
`package.json` → `npm test`, etc. — the same detection `Session::detect_project_commands` uses for
autofix, but against an arbitrary path since each candidate's worktree is a different directory);
cost via `Store::session_cost`. Guards are returned alive (not dropped) — the caller owns picking a
winner. `duel::merge_winner` is a thin wrapper over `worktree::merge_worktree_back`.

**Store (`forge-store`)** — `duel_outcome(id, repo_key, model, won, task, created_at)`
(migration 0002 in this build — forge-store's `MIGRATIONS`/`SCHEMA_VERSION` only had one entry
before this feature; if another migration lands first, renumber). `record_duel_outcome` inserts one
row per candidate per duel. `duel_boosts(repo_key)` aggregates `SUM(won)`/`COUNT(*)` per model into
`boost = ((wins - losses) as f64 * 0.5).clamp(-2.0, 2.0)`.

**Wiring** — `/duel <task>` → `CommandAction::Duel` → `DispatchOutcome::RunDuel { task }` →
`spawn_duel` (background task, same busy/spinner/interrupt machinery as a turn) →
`Session::run_duel` (builds the same `AgentCtx` shape `run_workflow`/`spawn_agents` do, converts
`Lifecycle` into the existing `Subagent*` presenter events) → `duel::run`. The finished
`(DuelReport, Vec<WorktreeGuard>)` can't travel back through a `JoinHandle<()>`, so it's written
into a `pending_duel: Arc<Mutex<PendingDuel>>` slot the done-signal drain picks up, opening
`PickerKind::Duel` and holding the report+guards in `duel_state` until the user resolves it.
Router construction (`build_provider_and_router`) loads `Store::duel_boosts(repo_key)` — the
canonicalized cwd's display string — and threads it in via `with_repo_boosts`; callers without a
store (e.g. a fresh `mcp_serve` path with none open yet) pass an empty map, a pure no-op.

## 5. Limitations
- Needs a git repo (`worktree::is_git_repo`) and at least 2 usable distinct-provider models — either
  missing is a hard error, not a silent single-model fallback.
- The winner's merge can conflict, exactly like any subagent worktree merge-back
  (`MergeReport::conflicted_files`); the note surfaces the conflicted file list, same as
  `spawn_agents`/workflow merges.
- Test detection is zero-config and best-effort — an unrecognized project type reads as "–" (no
  test signal), not a false pass or fail.
- Routing boosts are a soft nudge (bounded ±2.0 with a stable reorder), not a hard pin — they can be
  outweighed by other routing signals (budget pressure, quota, benched models).

## 6. Definition of done
- [x] `Router::route_candidates` (default + `HeuristicRouter` override); `with_repo_boosts` reorder.
- [x] `ResolvedAgent.pinned_model`; `route_child` bypasses the mesh for a pinned model.
- [x] `forge-core::duel` module: concurrent multi-model run, diffstat, test detection, cost.
- [x] `forge-store` migration + `record_duel_outcome`/`duel_boosts`.
- [x] `/duel` command, `PickerKind::Duel`, merge-on-pick / discard-on-cancel wiring.
- [x] Unit tests: parse arm, route_candidates (distinct-provider + default-impl fallback), pinned
      model bypass, boost reorder, duel_outcome roundtrip + boost math, diffstat parser.
- [x] `cargo fmt` + `clippy --workspace --all-targets` + `cargo test --workspace` clean.
