# Code archaeology: Model Mesh routing context

## Summary

`RoutingContext` keeps bounded, task-focused history for referential turns while
`SessionAffinity` carries cache warmth from one live session. These types are
load-bearing routing inputs: Core builds the context before it appends the
current turn, the optional LLM classifier consumes its bounded prompt, and the
heuristic router uses the same material to preserve tier and code-heavy floors.
The mechanical extraction to `forge-mesh/src/context.rs` preserves their root
imports and does not change the context limits, parsing, affinity state, or
route ordering.

## Timeline

- **2026-07-23, `d084b813` / PR #876:** introduced bounded task anchors,
  refinements, assistant status, compaction summaries, contextual routing, and
  prompt-derived continuation hints. This prevents isolated `continue` turns
  from losing the active task's capability requirements without injecting the
  whole transcript.
- **2026-07-24, `4d2007b2` / PR #879:** restricted classification to the
  active user task and current tool result rather than standing messages or
  unrelated history. This protects classifier correctness and stable cache
  inputs.
- **2026-07-28, `9a40bbb0` / PR #930:** added `SessionAffinity` and reusable
  cold-prefix estimates for continuation routing. The affinity decision must
  remain session-local and yield to measured quality, health, quota, context,
  and task-class constraints.

## Load-bearing invariants

1. **Bounded, task-focused context.** `from_messages` excludes UI-only chrome,
   tracks only one bounded task anchor plus recent refinements, and preserves a
   compaction summary only when it is the sole anchor. Unrelated new tasks must
   classify independently.
2. **Classifier safety and cache stability.** `classifier_prompt` labels prior
   text untrusted and caps all supplied fields. It must not turn transcript
   instructions into classifier instructions or expand context unpredictably.
3. **No contextual downgrade.** The active task material feeds deterministic and
   optional classification paths, preserving the floor for code-heavy and
   complex work.
4. **Session-local affinity.** The router is shared; affinity is supplied per
   decision and never persisted in the router. A cold-prefix estimate is not a
   cache-hit claim and never crosses models/providers.
5. **Shared execution/inspection behavior.** The same context is passed into
   route selection and explanation-related tests, so moving it cannot alter
   candidate ordering, failover, or rationale.

## Evidence and verification

- ADR-0006 requires deterministic, transparent routing.
- Focused Mesh tests cover compaction anchoring, bounded classifier prompts,
  independent tasks, active tool results, six-turn affinity replay, health,
  quota, context, and failover overrides.
- Core LLM-router tests cover bounded role-labelled prompts, contextual floors,
  cache keys, and affinity wrapper behavior.
- `long_session_endurance` covers repeated compaction, interruption recovery,
  and isolation.

## Safe change

Move this cohesive context/affinity owner behind a private module while
re-exporting `RoutingContext` and `SessionAffinity` from `forge_mesh`. Keep
policy constants and parsing bodies unchanged; only crate-private accessors
replace direct private-field use by affinity selection.

## Do not change in this phase

Context bounds, continuation detection, compaction marker, active-task
selection, affinity quality thresholds, candidate ordering, route explanation,
or the public root import paths. Those are behavioral policies, not extraction
details.
