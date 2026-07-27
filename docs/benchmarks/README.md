# Forge benchmarks

Forge benchmarks compare harnesses with the same underlying models, matched
tasks, explicit accounting, and official evaluators wherever available.

## Latest result: history-safe pinned and full-mesh auto

The canonical quota-bounded 2026-07-27 study used two SWE-bench Verified tasks,
regular `high` effort for pinned comparisons, one-commit history-isolated
repositories, trace audits, and the official `swebench==4.1.0` evaluator.

| Comparison | Quality | Wall-time result | Raw-token result | Cache-adjusted result |
|---|---:|---:|---:|---:|
| Forge pinned vs. native Codex, 6 pairs | **3/6 vs. 0/6** | **24.78% faster** | **55.61% lower** | **21.98% lower** |
| Forge pinned vs. native Claude Code, 4 pairs | **2/4 vs. 1/4** | **16.80% faster** | **45.45% lower** | **42.87% lower** |
| Full-mesh auto vs. five-pair average | **1/2; equal best native** | **44.26% faster** | **69.45% lower** | **56.33% lower** |

Mesh ran once per unique task with regular auto discovery, orchestration, and
failover enabled—no model pin and no effort override. It was faster than every
native model pair. The report also discloses the main exception: mesh used
12.45% more cache-adjusted tokens than native Terra on these two tasks.

Read the **[canonical report](history-safe-pinned-mesh-2026-07.md)** and the
**[83-cell validity ledger](cell-validity-2026-07.md)**.

The earlier [Claude 5 report](claude-code-claude5-2026-07.md) and
[GPT-5.6 report](codex-oauth-gpt56-2026-07.md) are retained as historical
artifacts, but their former headlines are superseded where repositories were
not history-isolated or Codex used `xhigh`.

## Benchmark documentation

- [Measured-results history](results.md) — earlier results, corrections, and
  honest caveats.
- [History-safe pinned and mesh-auto result](history-safe-pinned-mesh-2026-07.md) —
  current same-model and routing evidence.
- [Cell-validity ledger](cell-validity-2026-07.md) — every retained formal
  cell, official outcome, integrity finding, and exclusion reason.
- [SWE-bench guide](swe-bench.md) — reproduce comparisons with `forge bench`.
- [Why Forge is a better harness](../harness/why-forge-is-a-better-harness.md) —
  broader test-backed rationale and limitations.

Headline results are specific to their recorded model, sample, configuration,
and date. They should not be generalized to every model or workload without a
matched run.
