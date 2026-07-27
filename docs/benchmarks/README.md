# Forge benchmarks

Forge benchmarks compare harnesses with the same underlying models, matched
tasks, explicit accounting, and official evaluators wherever available.

## Latest result: Forge vs. native Codex on GPT-5.6

The fresh, predeclared 2026-07-26–27 run used GPT-5.6 Sol, Terra, and Luna at
`xhigh` reasoning on 18 stratified SWE-bench Verified tasks.

| Metric | Forge | Native Codex | Result |
|---|---:|---:|---|
| Official Docker resolves | **18 / 18** | **18 / 18** | Quality parity |
| Generation wall time | **3,299.841s** | 5,574.566s | **Forge 40.81% faster** |
| Whole-session tokens | **9,692,604** | 24,643,373 | **Forge 60.67% lower** |
| Faster matched pairs | **13 / 18** | 5 / 18 | Forge advantage |
| Lower-token matched pairs | **17 / 18** | 1 / 18 | `p=0.00014496` |

All six official reports completed with zero evaluator errors. See the
**[complete GPT-5.6 Forge vs. native Codex report](codex-oauth-gpt56-2026-07.md)**
for the controlled protocol, per-model results, artifact paths, caveats,
statistical checks, and pre-optimization comparison.

## Benchmark documentation

- [Measured-results history](results.md) — earlier results, corrections, and
  honest caveats.
- [SWE-bench guide](swe-bench.md) — reproduce comparisons with `forge bench`.
- [Why Forge is a better harness](../harness/why-forge-is-a-better-harness.md) —
  broader test-backed rationale and limitations.

Headline results are specific to their recorded model, sample, configuration,
and date. They should not be generalized to every model or workload without a
matched run.
