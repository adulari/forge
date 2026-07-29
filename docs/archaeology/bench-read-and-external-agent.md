# Code archaeology: benchmark read side and external agent

## Boundaries

Two owners split out of the SWE-bench harness, along the seam between *producing* predictions and
everything else:

- `bench/report.rs` — the read side. `pass@k` across seeds, loading the `*.metrics.jsonl`
  sidecars, and joining them with the official evaluator's `resolved_ids` into the agent
  comparison table.
- `bench/external_agent.rs` — running a competing agent CLI (Claude Code / Codex) unattended on
  the same instance, plus best-effort usage extraction from its machine output.

`bench.rs` keeps the harness itself: instance loading, repo preparation, the Forge turn, patch
extraction, sweeps, and the prediction/metric artifacts.

## Interfaces

`passk` and `report` stay `pub(crate)` for `cli/dispatch.rs`; `load_metrics` narrowed to
`pub(super)` (its only non-report use is the harness's own round-trip test).
`run_external_agent` is `pub(super)` and keeps its five-field return, including the
`metrics_complete` flag.

## The invariant that made these worth separating

Both owners exist to keep one claim honest: Forge solves the same instances with fewer tokens.
`external_agent` reports `metrics_complete = false` rather than guessing when a CLI emits no
parseable usage, and `report`'s `tok_per_success_cell` refuses to print a tokens-per-success
number unless there are eval results, something resolved, and every row's capture was complete.
Those two rules are now adjacent to the code that establishes them.

## Characterization

The aggregation and usage-parsing tests moved with their code (`summarize_agent`,
`tok_per_success_cell` honesty conditions, claude/codex usage parsing including cache-token
handling and the garbage-input case) and pass unchanged.
