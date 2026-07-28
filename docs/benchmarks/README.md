# Forge benchmarks

Forge benchmarks compare harnesses on matched tasks with explicit quality, wall-time, token, quota,
and integrity accounting. Official evaluators are used where the benchmark provides one.

## Latest result: cache-aware matched long-session stress

The quota-bounded 2026-07-28 confirmation ran one continuous six-prompt Forge session against the
same history-isolated synthetic repository as the retained native Codex and native Claude cells:

| Comparison | Quality | Work wall time | Raw tokens | 25%-adjusted | Cache-zero |
|---|---:|---:|---:|---:|---:|
| Forge full mesh auto, cache-aware | **Passed** | **511.200s** | **996,529** | **363,697** | **152,753** |
| Native Codex Sol high | Passed | 1,333.317s | 5,129,177 | 1,430,681 | 197,849 |
| Native Claude Opus 5 high | Failed hidden conflict precedence | 1,019.400s | 4,641,031 | 1,362,289.75 | 269,376 |

Forge matched native Codex quality, was 61.66% faster, and used 80.57% fewer raw, 74.58% fewer
cache-adjusted, and 22.79% fewer cache-zero-credit tokens. Against clean native Claude, Forge passed
the hidden invariant Claude missed, was 49.85% faster, and used 78.53% fewer raw, 73.30% fewer
cache-adjusted, and 43.29% fewer cache-zero-credit tokens. Its whole harness arm improved on the
previous accepted Forge sample by 8.63%, while the like-for-like persisted user-to-terminal-response
boundary improved by 4.84% (472s versus 496s, one-second timestamp resolution). Raw tokens improved
by 41.34% and cache-zero sensitivity by 66.17%.
Honest review still excludes the original Claude result because its base tree tracked generated
bytecode; only the exact-tree replacement is current. The
**[long-session report](forge-long-session-stress-2026-07.md)** retains exact routes, acceptance,
integrity, quotas, all optimization attempts, fixes, and minimum retest evidence.

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
- [Cache-aware Forge confirmation](artifacts/long-session-forge-cache-affinity-2026-07.json) —
  exact per-turn routes, affinity decisions, tokens, timing, quota observations, acceptance, and
  optimization-attempt ledger.
- [Strengthened long-session replay ledger](artifacts/long-session-hidden-replay-2026-07.json) —
  verifier and patch hashes plus the free interleaving/cancellation-rollback re-evaluation.
- [Native Codex token recalculation](artifacts/long-session-codex-token-recalculation-2026-07.json) —
  source hashes, cumulative snapshots, per-turn deltas, and corrected comparison totals.
- [Clean native Claude ledger](artifacts/long-session-native-claude-clean-2026-07.json) — exact
  setup, per-turn tokens/time, retained local failures, quality outcome, integrity, and quota.
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
