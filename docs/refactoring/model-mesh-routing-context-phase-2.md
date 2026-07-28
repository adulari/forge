# Model Mesh routing context refactor — phase 2

This mechanical extraction continues the canonical campaign in
`docs/architecture/codebase-size-analysis-2026-07.md` after the classification
pilot. It moves the bounded contextual-routing and session-affinity owner from
the Mesh crate root into a private deep module.

## Scope and non-change contract

`crates/forge-mesh/src/context.rs` owns `RoutingContext`, `SessionAffinity`,
context bounds, compaction-aware anchor selection, continuation detection, and
safe classifier prompt construction. The crate root deliberately re-exports
`RoutingContext` and `SessionAffinity`; Core and other callers retain their
existing imports. The move changes no public API, configuration, persisted
state, context text, route policy, candidate order, explanation, affinity
threshold, or provider request.

The module is a deep owner rather than a forwarding layer: it owns the data,
construction, bounded rendering, and continuation invariants together. Its
only narrow dependency on classification is the existing trivial-pattern
regression guard needed by continuation recognition.

## Archaeology and risks

See `docs/archaeology/model-mesh-routing-context.md`. The highest-risk
properties are bounded task-focused inputs, untrusted classifier context,
no-downgrade handling for dependent coding work, and session-local cache
warmth. The move retains the original bodies and adds only crate-private
accessors for affinity selection, avoiding a wider internal representation.

## Result

| Measure | Before | After | Result |
|---|---:|---:|---|
| `forge-mesh/src/lib.rs` implementation lines | 1,742 | 1,420 | Lower root concentration |
| `forge-mesh/src/context.rs` implementation lines | — | 326 | Under the 500-line target |
| Workspace implementation files ≤500 | 116/181 | 117/182 | Improved |
| Workspace implementation files ≤800 | 146/181 | 147/182 | Improved |
| Files above 2,000 / 5,000 / 10,000 implementation lines | 9 / 4 / 1 | 9 / 4 / 1 | No regression |

The baseline is commit `80c0da37f007e6f5858ca66f3c7cbd6fea673529`, clean
before this phase. Aggregate Rust implementation lines rise only by module and
visibility scaffolding; no dependency or public interface is added.

## Verification

- `cargo fmt --all -- --check`
- `cargo clippy --locked -p forge-agent-mesh --all-targets --all-features -- -D warnings`
- `cargo test --locked -p forge-agent-mesh --lib --all-features` — 211 passed, 1 ignored
- `cargo test --locked -p forge-agent-core llm_router::tests --all-features` — 16 passed
- `cargo test --locked -p forge-agent-core --test long_session_endurance --all-features` — 3 passed
- `python3 scripts/ci/architecture_size.py` — passed

No model benchmark is required: this architecture-only movement does not change
model-visible context, request ordering, routing policy, persistence, or a
provider runtime path.
