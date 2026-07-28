# Model Mesh classification refactor — phase 1

This is the first implementation phase of
`docs/architecture/codebase-size-analysis-2026-07.md`. That study remains the
canonical campaign plan; this document records only concrete decisions for the
Model Mesh pilot.

## Target state

`forge-mesh` retains one public `Router` interface and one `RoutingDecision` used by
execution and inspection. The crate root continues to own `HeuristicRouter`, route
orchestration, ranking, quota/health filtering, affinity, and failover. A private
classification module owns:

- weighted prompt signals and matching helpers;
- the internal classification result;
- public-compatible `RouteHints`;
- deterministic `score_prompt`;
- the `max_tier` no-downgrade rule.

The crate root deliberately re-exports `RouteHints` and `max_tier`, preserving all
existing callers. The module is deep because it owns a policy family, its invariants,
and focused tests; it is not a pass-through and introduces no new trait or crate.

```text
forge_mesh crate root
├── Router / HeuristicRouter / RoutingDecision
├── routing context, candidate selection, quota, affinity, failover
├── classification (private)
│   ├── weighted signals and prompt matching
│   ├── Classification / RouteHints
│   ├── score_prompt
│   └── max_tier
└── explain
    └── consumes the same classification result as execution
```

## Non-change contract

- No external API, configuration, persisted type, or database schema change.
- No classifier weight, threshold, signal, reason, mode, prompt, or cache change.
- No candidate ordering, quota, health, affinity, cold-prefix, failover, pin, or
  rationale behavior change.
- No model-visible prompt/context/tool-schema change and therefore no paid provider
  benchmark.
- No new crate dependency, public trait, or adapter.
- Preserve `forge_mesh::RouteHints` and `forge_mesh::max_tier`.

## Baseline

- Commit: `9a40bbb0551e2dde96f895b75267da63882a139a`.
- Worktree state: clean before changes.
- `crates/forge-mesh/src/lib.rs`: 5,588 physical lines. It contains two inline
  `#[cfg(test)]` modules; production and test lines must be measured separately.
- Direct dependencies remain `forge-types`, `forge-config`, `async-trait`, `serde`,
  and `serde_json`; no dependency-cycle or crate-boundary change is planned.
- Clean focused check: 12.14 s.
- Immediate incremental focused check: 0.17 s.
- Focused lib suite: 211 passed, 1 ignored; reported execution 0.43 s.
- Runtime baseline: README and the retained PR #930 evidence in
  `docs/benchmarks/forge-long-session-stress-2026-07.md`,
  `docs/benchmarks/artifacts/long-session-forge-cache-affinity-2026-07.json`, and
  `docs/benchmarks/cell-validity-2026-07.md`.

## Refactoring sequence

Every step must leave the repository buildable and independently revertible.

1. **Install measurement and no-growth guards.**
   - Add a deterministic owned-Rust source measurement script.
   - Report physical production and test lines separately.
   - Freeze existing files above the 800-line implementation target so they cannot
     grow silently.
   - Reject new implementation files above 800 lines.
   - Prevent regression in the 500/800-line distribution and counts above
     2,000/5,000/10,000 lines.
   - Ratchet existing crate-root and hotspot exceptions rather than resetting the
     baseline after movement.
   - Add script tests and run the gate in CI.

2. **Freeze behavior characterization.**
   - Identify the existing labeled corpus, prompt-boundary, self-hosting,
     contextual follow-up, LLM no-downgrade, inspection/execution, affinity,
     failover, pin, budget, health, and quota tests as the safety net.
   - Run the focused Mesh suite before movement.
   - Add a test only if a public compatibility or shared-decision invariant lacks
     coverage; do not test private line placement.

3. **Mechanically extract classification.**
   - Create a private `classification` module.
   - Move constants, result/hint types, matching helpers, scoring, and `max_tier`
     without changing their bodies.
   - Use the moved `score_prompt` from both execution and `explain`.
   - Re-export only the two existing public items.
   - Run formatting, focused check, and focused tests.

4. **Move focused tests toward their owner.**
   - Move self-contained pure-classification tests only when they do not require
     duplicating broad router fixtures.
   - Leave cross-policy integration tests at the crate boundary.
   - Run focused tests again.

5. **Update source-linked documentation.**
   - Update normative symbol/line references only after final movement.
   - Run documentation sync tests.

6. **Measure and review.**
   - Compare production/test LOC distribution with the frozen baseline.
   - Repeat clean and incremental focused checks under the same conditions.
   - Run the complete required verification sequence.
   - Perform an honest final-diff review and fix every supported finding before
     publication.

## Test strategy

Focused safety net includes:

- labeled-corpus accuracy and weighted-signal unit tests;
- word-boundary and trivial-pattern regressions;
- self-hosting versus unrelated-project classification;
- contextual/referential follow-up and compaction-summary replay;
- bounded classifier prompt and active-user-task filtering;
- weak-LLM no-downgrade and code-heavy floor tests in Core;
- inspection/execution ordering equivalence;
- affinity quality, cold-prefix, health, quota, context, and failover overrides;
- pin, budget, candidate-chain, and provider-distinctness tests.

Verification order:

1. architecture guard script tests and current-tree check;
2. `cargo fmt --all -- --check`;
3. `cargo clippy --locked -p forge-agent-mesh --all-targets --all-features`
   with warnings denied;
4. `cargo test --locked -p forge-agent-mesh --lib --all-features`;
5. affected Core LLM-router and Mesh explanation/document-sync tests;
6. relevant local endurance/replay tests;
7. workspace checks and tests because crate-root visibility is shared;
8. `cargo build --release --locked --bin forge`.

Paid model benchmarks are explicitly excluded unless review finds an actual change to
model-visible context, request ordering, persistence, or runtime routing behavior.

## Risk assessment

| Risk | Level | Control |
|---|---|---|
| Privacy change makes execution and inspection use different scoring paths | High | One crate-private `score_prompt`; focused explanation parity test |
| Public import break for Core | High | Root re-exports plus workspace check |
| Accidental signal/threshold/rationale edit during movement | High | Move bodies mechanically; inspect move-aware diff; full classifier suite |
| Context/affinity hint regression | High | Preserve `RouteHints`; contextual and six-turn replay tests |
| Guard misclassifies inline tests as implementation | Medium | Lexer-aware script tests with inline and external test fixtures |
| Shallow module merely lowers root line count | Medium | Module owns constants, behavior, API-compatible hints, and focused tests |
| Compile iteration regression | Low | Same-condition clean/incremental before/after measurement |

## Rollback plan

- The guardrail commit can ship independently.
- Behavior-characterization changes can ship independently.
- If extraction or privacy changes fail, revert the mechanical extraction while
  retaining guards and tests.
- If test movement obscures ownership or creates fixture duplication, retain tests
  at the crate boundary and defer that movement.
- Stop before any policy or compatibility cleanup; those require separate later
  phases.

## Completed result

Measured against the frozen `9a40bbb0551e2dde96f895b75267da63882a139a`
baseline:

| Measure | Baseline | After phase | Result |
|---|---:|---:|---|
| Owned Rust implementation files | 180 | 181 | One deep policy owner added |
| Owned Rust implementation lines | 123,760 | 123,775 | +15 lines of module/visibility scaffolding |
| Owned Rust test lines | 65,588 | 65,593 | +5 lines of test-module scaffolding |
| Implementation files ≤500 lines | 115/180 (63.9%) | 116/181 (64.1%) | Improved |
| Implementation files ≤800 lines | 145/180 (80.6%) | 146/181 (80.7%) | Improved |
| Implementation files >2,000 lines | 10 | 9 | Improved |
| Implementation files >5,000 lines | 4 | 4 | No regression |
| Implementation files >10,000 lines | 1 | 1 | No regression |
| `forge-mesh/src/lib.rs` physical lines | 5,588 | 4,931 | Below the 5,000 physical-line milestone |
| `forge-mesh/src/lib.rs` implementation lines | 2,214 | 1,742 | 21.3% lower concentration |
| `classification.rs` implementation lines | — | 486 | Inside the 500-line target |
| `classification/tests.rs` test lines | — | 188 | Tests remain visible and separately counted |

The increase in aggregate lines is reported rather than hidden: this phase improves
ownership and concentration, not repository size. The new module passes the deletion
test—removing it would spread classification constants, matching, scoring, public
hints, and no-downgrade policy back across the router and inspector.

### Performance and dependency evidence

The before/after checks used fresh Cargo target directories and the same command:
`cargo check --locked -p forge-agent-mesh --all-features`.

| Check | Baseline | After phase |
|---|---:|---:|
| Clean focused check | 12.14 s | 12.20 s |
| Immediate incremental focused check | 0.17 s | 0.17 s |
| Focused test execution | 0.43 s | 0.43 s |
| Focused test build/run command wall time | 16.47 s | 15.55 s |

These are single local samples. The clean-check difference is approximately 0.5%
and is treated as neutral; the lower test command time is recorded without claiming
the extraction caused it. The crate dependency graph is unchanged and no dependency,
trait, crate, protocol, persistence type, or public import path was added. The locked
release build produced a 97,686,512-byte binary in 2m52s; no baseline binary-size
claim is made because this phase added no dependency.

### Verification evidence

- Architecture guard unit tests: 7 passed.
- Architecture guard against current tree: passed.
- `cargo fmt --all -- --check`: passed.
- Workspace warnings-denied Clippy across all targets/features: passed.
- Focused Mesh lib suite: 211 passed, 1 ignored, unchanged from baseline.
- Workspace all-features suite: 2,372 passed, 27 ignored across 49 suites.
- Manual-E2E/harness unit suite: 45 passed, covering retained capture,
  terminal-turn, quota/integrity, and summarization machinery.
- Changed-file and runner-resource CI contract tests: passed.
- `cargo build --release --locked --bin forge`: passed.

No paid model benchmark ran. The extraction changed no model request, prompt, context,
tool schema/order, routing policy, persistence behavior, or provider runtime path, so
the retained PR #930 benchmark evidence remains the applicable runtime baseline.

### Scorecard assessment

- **Module size and depth:** root concentration falls materially; the new 486-line
  owner is under the strict target and owns behavior rather than forwarding calls.
- **Dependency/interface health:** module is private; existing public
  `forge_mesh::RouteHints` and `forge_mesh::max_tier` paths are deliberate
  re-exports; no speculative trait or dependency was introduced.
- **Change locality:** classifier signals, matching, scoring, and focused tests now
  have one discoverable owner. `explain` explicitly consumes that same scorer.
- **Verification quality:** pure classification corpus/self-hosting tests moved to
  the owner; cross-policy routing, affinity, quota, and failover tests remain at the
  crate boundary.
- **Runtime product performance:** compile/test iteration is neutral and all retained
  behavior/replay suites pass.
- **AI navigability:** the owning file is named for the domain concept, under 500
  implementation lines, documented at module level, and referenced by the normative
  routing guide.
