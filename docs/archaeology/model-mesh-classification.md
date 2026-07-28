# Code archaeology: Model Mesh task classification

## Summary

Model Mesh classification is a cohesive policy family inside
`crates/forge-mesh/src/lib.rs`: prompt signals produce a `TaskTier`, confidence score,
and rationale; `RouteHints` carries prompt-derived routing context; and `max_tier`
prevents contextual or LLM classification from downgrading a deterministic floor.
The code is safe to move mechanically, but not safe to simplify during the move.

The load-bearing boundary is that route execution and route inspection consume the
same classification implementation. `crates/forge-mesh/src/explain.rs` calls
`score_prompt`, while `HeuristicRouter` uses the same function before `decide`.
`forge-core/src/llm_router.rs` consumes the public `RouteHints` and `max_tier`
compatibility surface. The extraction must preserve those paths without introducing
another classifier, decision type, or speculative trait.

## Evidence base

- Base commit: `9a40bbb0551e2dde96f895b75267da63882a139a` (clean worktree).
- Architectural decisions:
  - ADR-0006 requires deterministic, transparent, configurable routing with a
    recorded rationale and no mandatory classifier call.
  - ADR-0011 makes benchmark data optional and retains heuristic fallback.
- Normative behavior: `docs/features/mesh-routing.md`.
- File history: 51 commits touch `crates/forge-mesh/src/lib.rs`; its most frequent
  co-change is `crates/forge-core/src/lib.rs` (34 commits), followed by
  `catalog.rs` and config (23 each), `forge-core/src/llm_router.rs` (17), and
  `explain.rs` (15). Classification is therefore a useful locality boundary.
- Baseline verification:
  - `cargo check --locked -p forge-agent-mesh --all-features`: clean 12.14 s;
    immediate incremental 0.17 s.
  - `cargo test --locked -p forge-agent-mesh --lib --all-features`: 211 passed,
    1 ignored; reported test execution 0.43 s.

## Timeline

### 2026-06-15 — weighted deterministic scoring and optional LLM classification

Commit `5fb99869` / PR #30 replaced a length-dominated classifier with weighted
signals. It made prompt length a capped nudge, allowed short difficult prompts to
classify as Complex, preserved trivial mechanical edits, recorded firing signals,
and split classification from the public synchronous `decide` path so optional LLM
classification could reuse deterministic selection.

**Still applies:** yes. The score, reasons, and deterministic fallback form the
auditability contract from ADR-0006.

### 2026-06-18 — hybrid classification and explainability

Commit `b057ddb0` / PR #112 introduced a hybrid mode: confident heuristic results
avoid an LLM call, uncertain results may consult one, and inspection reports the
effective classifier. This reinforced the need for a single reusable heuristic
classification owner.

**Still applies:** yes. Mechanical movement must not alter classifier mode,
confidence boundaries, fallback, or inspector output.

### 2026-07-01 — regression hardening and project context

- Commit `5dbd2def` / PR #427 added word-boundary matching after `port` matched
  `report`/`export` and `test` matched `latest`/`contest`.
- Commit `0aae4459` / PR #441 added a labeled prompt corpus, made
  `step-by-step` an explicit Complex hint, strengthened the trivial-edit penalty,
  and expanded action verbs.
- Commit `acba9bc9` / PR #442 gated self-hosting infrastructure terms on
  `ProjectContext::is_self_hosting` and fixed inspection/execution consistency.

**Still applies:** yes. Word boundaries, self-hosting gating, score thresholds,
and human-readable reasons are regression fixes, not incidental helpers.

### 2026-07-12 to 2026-07-14 — weak-classifier containment

Commit `5ddbb30a` / PR #648 added health-aware classifier candidates, cache/fallback
behavior, and a no-downgrade guard. Commit `e237301c` / PR #761 made code-editing
tasks at least Standard and broadened code-heavy detection after weak free models
under-labeled real editing work.

**Still applies:** yes. `RouteHints::code_heavy` and `max_tier` are public because
Core's optional LLM classifier uses them to preserve the deterministic floor.

### 2026-07-23 to 2026-07-24 — bounded active-task context

Commit `d084b813` / PR #876 added bounded `RoutingContext` so referential turns such
as “continue” retain the active task's tier and code-heavy seed without exposing the
whole transcript. Commit `4d2007b2` / PR #879 then restricted classification to the
active user task and its current tool results, excluding standing system/developer
messages and unrelated earlier turns.

**Still applies:** yes. `RouteHints::from_context`, active-task material, bounded
classifier prompts, and no-downgrade tier combination preserve context compaction
and cache behavior.

### 2026-07-28 — cache-aware session affinity

Commit `9a40bbb0` / PR #930 extended `RouteHints` with continuation and adversarial
review signals used by session-affinity policy. It also added retained replay tests
covering route inspection versus execution order and failover recovery.

**Still applies:** yes. The classification extraction must preserve the exact
`RouteHints` layout and construction behavior so affinity, cold-prefix reasoning,
and rationale remain unchanged.

## Key decisions and invariants

### One classifier feeds execution and inspection

**What it does:** `score_prompt` produces the tier, score, and reasons used by both
`HeuristicRouter` and `explain.rs`.

**Why:** route explanations must describe the decision that execution would make,
not a parallel approximation.

**Evidence:** ADR-0006, PR #442, `explain.rs`, and
`session_affinity_tests::contextual_route_inspection_matches_execution_order`.

**Extraction rule:** move the function once and use it from both callers. Do not
copy or wrap it in a second policy implementation.

### Classification can raise a floor but must not weaken it

**What it does:** `max_tier` combines deterministic, contextual, and optional LLM
results. Code-heavy tasks receive a Standard floor.

**Why:** weak classifier models previously labeled genuine code work Trivial.

**Evidence:** PRs #648 and #761; `forge-core/src/llm_router.rs`.

**Extraction rule:** preserve the public function and `RouteHints` via deliberate
re-exports from the crate root.

### Prompt matching is deliberately defensive

**What it does:** whole-word and boundary-aware matching avoids substring false
positives; trivial patterns deliberately outweigh one ambiguous reasoning term;
short factual HTTP status explanations receive a narrow exception.

**Why:** each rule corresponds to a reported classification regression.

**Evidence:** PRs #427 and #441 plus focused tests named for `report`/`export`,
`latest`/`contest`, trivial patterns, depth hints, and HTTP status explanations.

**Extraction rule:** move constants and helpers together. Do not rename, reorder,
deduplicate, or tune the policy in this phase.

### Context is bounded and task-focused

**What it does:** contextual hints use bounded active-task material, preserve
referential continuations, and exclude unrelated standing messages.

**Why:** isolated “continue” prompts lost capability requirements, while feeding
whole histories would destabilize cache keys and expose irrelevant text.

**Evidence:** PRs #876 and #879 and contextual routing regression tests.

**Extraction rule:** `RouteHints` may call back into `RoutingContext`, but the new
module must not own transcript persistence or change context limits.

## Safe changes

- Move classification constants, `Classification`, `RouteHints`, prompt-matching
  helpers, `score_prompt`, and `max_tier` into one private module.
- Give the parent module only the visibility required to call `score_prompt` and
  inspect its result.
- Re-export `RouteHints` and `max_tier` at their existing crate-root paths.
- Update source-linked documentation after code movement.
- Move focused pure-classification tests toward the owning module only after the
  mechanical extraction passes unchanged.

## What to leave alone

- Tier thresholds, signal weights, string lists, matching semantics, and rationale
  text.
- `RoutingContext` bounds and active-task selection.
- `HeuristicRouter::decide`, candidate construction, health/quota filtering,
  affinity, failover ordering, and explanation decision construction.
- Public API paths used by `forge-core`.
- Classifier mode defaults, LLM prompts, caches, timeouts, and provider calls.

## Unknowns to verify during review

- Whether any documentation sync test assumes exact `lib.rs` line numbers rather
  than symbol ownership.
- Whether moving `score_prompt` changes privacy assumptions in `explain.rs`.
- Whether compile/test iteration stays neutral after the additional module.

