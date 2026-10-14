# Bench metrics: per-run scoring and cost per solved task

`forge bench report` turns `bench swe` metrics sidecars and the official SWE-bench evaluation
reports into a per-agent comparison. This page records what each number means and which rules
keep it honest. The end-to-end benchmark flow is in
[docs/benchmarks/swe-bench.md](../benchmarks/swe-bench.md).

## Pairing runs with their own eval report

A *run* is one metrics sidecar (one agent, one seed). Each run is scored only by its own report:

- `--run <metrics.jsonl>=<eval.json>` (repeatable) names the pair explicitly.
- `--metrics` / `--eval` without `--run` pair by position when the counts match. A count mismatch
  is an error rather than a guess.
- `--shared-eval` keeps the old behaviour: the union of every `--eval` is applied to every run.
  Use it only for one combined report; it credits a run with solves from the others.

Before this change the report always unioned every eval, so an instance only Forge solved was
also counted as solved for Codex, and every seed's pass@1 was really pass@k.

## Groups and metrics

Runs are grouped by `agent` plus `runtime.resolved_model` (else the requested model). Per group:

| column | meaning |
|---|---|
| `k` | attempts (runs) in the group |
| `n` | instances per attempt |
| `pass@1` | mean over attempts of solved ÷ eligible, ± sample stdev (n − 1) when k ≥ 2 |
| `pass@k` | instances solved by any attempt ÷ instances eligible in any attempt |
| `tok/att` | total tokens ÷ attempt rows with complete capture |
| `cache` | cached input ÷ input, over rows that reported a cached count |
| `$/att` | recorded spend ÷ complete rows |
| `$/solved` | `$/att` ÷ mean pass@1 (the Coding Agent Index definition) |
| `ntl$/att`, `ntl$/solv` | the same with notional cost (below) |
| `tok/success` | total tokens ÷ solved attempts, only when every row's capture was complete |

Eligibility and scoring rules (`crates/forge-cli/src/bench/scoring.rs`):

- Rows with `tools_unavailable` (or `outcome = tools_unavailable`) are left out of pass@1: Forge's
  tools never started, so the attempt is an infrastructure error, not a miss.
- Rows with `outcome = refused` or `integrity_fail` score 0 even if the evaluator resolved them,
  and stay in the denominator.
- Cost and token sums cover only `metrics_complete` rows; the footnote under the table says how
  many rows were left out.

## Pareto block

A dominated group has another group with lower-or-equal cost per attempt and higher-or-equal mean
pass@1, strictly better on one. The block uses notional cost when every group has it, else
recorded cost, and says which. It is refused when any attempt of any group has fewer than 20
pass@1-eligible instances (toolless attempts do not count), because cost per solved task on a
handful of instances is noise. It is also refused when recorded cost is the basis and a group
recorded $0, since that is a subscription or free route rather than a free solve.

## Per-instance fields

All new `InstanceMetric` fields are `#[serde(default)]`, so older sidecars load unchanged.

- `attempt`: 1-based seed; 0 in old sidecars.
- `session_id`: the Forge session that ran the attempt (Forge agent only).
- `cached_input_tokens`, `cache_write_tokens`, `reasoning_tokens`: subsets of input/output.
  `None` means the provider did not report the split.
- `model_calls`, `tool_calls`: provider calls and tool calls in the session (Forge agent only).
- `notional_cost_usd`: see below.
- `outcome`, `outcome_detail`: `ok`, `refused`, `integrity_fail`, `tools_unavailable`,
  `timed_out`. `integrity_fail` is reserved for the post-hoc integrity checks.

For the Forge agent these come from `Session::session_usage_detail()`
(`crates/forge-core/src/session_usage_detail.rs`), backed by `Store::session_usage_by_model` and
`Store::session_tool_call_count`. Like `session_usage_db`, it counts every call in the session,
including side calls and deactivated turns.

## Notional cost

About a quarter of real usage rows record `cost_usd = 0` (subscription bridges, free routes), so
recorded spend cannot compare those arms. `notional_cost_usd` is the recorded cost plus every
zero-cost call priced at its model's list rate through `Pricing::reference_cost_for_usage`
(exact id, else the cheapest positive rate for the same bare model name; cached input at the
cache-read rate, cache writes at the plain input rate). It is `None` when a zero-cost call that used
tokens has no tracked price, so an unpriced call is never counted as $0. Bridge aliases such as
`claude-cli::sonnet` only get a price when some namespace tracks the same bare model name. For Claude Code, `total_cost_usd` is already a list-price figure and is
used as-is; Codex reports no cost, so its notional cost is `None`. The report always labels these
columns notional; they are never spend.

## Refusals

Provider stop reasons do not reach `ModelResponse`, so refusals are detected from two signals and
`outcome_detail` says which one fired (`crates/forge-cli/src/bench/outcome.rs`):

- the turn failed with the provider's content-filter error, or
- the patch is empty and the final answer contains a first-person refusal phrase ("I can't help
  with", "I must decline", ...). This one is a heuristic and is labelled as such.

## Token-detail columns on `usage`

Store migration 42 adds nullable `usage.cache_write_tokens` and `usage.reasoning_tokens`. NULL
means the provider did not report the split. They are filled where providers report them:

- genai providers (Anthropic, Bedrock, OpenAI-compatible): `prompt_tokens_details.cache_creation_tokens`
  and `completion_tokens_details.reasoning_tokens`. genai maps a zero to "not reported".
- Claude Code bridge: `cache_creation_input_tokens`.
- Codex bridge: `cache_write_input_tokens` and `reasoning_output_tokens`.
- codex-oauth / Responses API: `output_tokens_details.reasoning_tokens`.

Antigravity's `thinking_tokens` can exceed its `output_tokens`, so it is not recorded as a subset.
Migration 41 is reserved for the cache-instrument work and is empty on this branch.
