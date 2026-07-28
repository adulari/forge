# Forge benchmarks

Forge benchmarks compare harnesses on matched tasks with explicit quality, wall-time, token, quota,
and integrity accounting. Official evaluators are used where the benchmark provides one.

## Latest result: matched long-session stress

The quota-bounded 2026-07-28 study ran one continuous six-prompt session against the same
history-isolated synthetic repository for Forge and native Codex. A clean native Claude replacement
is pending authentication:

| Comparison | Quality | Wall time | Raw tokens | Cache-adjusted tokens |
|---|---:|---:|---:|---:|
| Forge full mesh auto | **Passed** | **559.469s** | **1,698,715** | **763,291** |
| Native Codex Sol high | Passed | 1,333.317s | 15,881,503 | 4,541,599 |
| Native Claude Opus 5 high | Clean rerun pending | — | — | — |

Forge matched native Codex quality, was 58.04% faster, and used 83.19% fewer cache-adjusted
tokens. Honest review excluded the original Claude result because its base tree tracked generated
bytecode; a correct-tree replacement stopped at zero model tokens after local OAuth expired. The
**[long-session report](forge-long-session-stress-2026-07.md)** retains the exception, exact routes,
acceptance, integrity, quota, both failed Forge confirmations, fixes, and minimum retest evidence.

## SWE-bench result: history-safe pinned and full-mesh auto

The canonical quota-bounded 2026-07-27 study used two SWE-bench Verified tasks, regular `high`
effort for pinned comparisons, one-commit history-isolated repositories, trace audits, and the
official `swebench==4.1.0` evaluator.

| Comparison | Quality | Wall-time result | Raw-token result | Cache-adjusted result |
|---|---:|---:|---:|---:|
| Forge pinned vs. native Codex, 6 pairs | **3/6 vs. 0/6** | **24.78% faster** | **55.61% lower** | **21.98% lower** |
| Forge pinned vs. native Claude Code, 4 pairs | **2/4 vs. 1/4** | **16.80% faster** | **45.45% lower** | **42.87% lower** |
| Full-mesh auto vs. five-pair average | **1/2; equal best native** | **44.26% faster** | **69.45% lower** | **56.33% lower** |

Mesh ran once per unique task with regular auto discovery, orchestration, and failover enabled—no
model pin and no effort override. It was 14.68% faster than the fastest native model pair. The
report also discloses the main exception: mesh used 12.45% more cache-adjusted tokens than native
Terra on the two tasks.

Read the **[canonical SWE-bench report](history-safe-pinned-mesh-2026-07.md)** and
**[83-cell validity ledger](cell-validity-2026-07.md)**.

## Historical reports

The [Claude 5 report](claude-code-claude5-2026-07.md) and
[GPT-5.6 report](codex-oauth-gpt56-2026-07.md) are retained as historical artifacts. Their former
headlines are superseded where repositories were not history-isolated or Codex used `xhigh`.

## Benchmark documentation

- [Matched long-session stress](forge-long-session-stress-2026-07.md) — native comparison,
  real-history workload shape, multi-prompt mesh, failed-attempt ledger, fixes, and limitations.
- [History-safe pinned and mesh-auto result](history-safe-pinned-mesh-2026-07.md) — current
  same-model SWE-bench routing evidence.
- [Cell-validity ledger](cell-validity-2026-07.md) — every retained formal cell, official outcome,
  integrity finding, and exclusion reason.
- [Measured-results history](results.md) — earlier results, corrections, and caveats.
- [SWE-bench guide](swe-bench.md) — reproduce official comparisons with `forge bench`.
- [Why Forge is a better harness](../harness/why-forge-is-a-better-harness.md) — broader
  test-backed rationale and limitations.

Headline results are specific to the recorded model, sample, configuration, account state, and
date. They should not be generalized to every model or workload without a matched run.
