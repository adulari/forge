# History-safe Forge vs. native CLIs and mesh auto

**Canonical result — 2026-07-27.** This report supersedes the July headline
tables that used non-isolated repository history or mismatched `xhigh` effort.
It compares the same models at regular `high` effort for pinned runs, then
compares genuine unpinned Forge mesh auto against every clean native model pair.

The quota-bounded sample is deliberately small: two SWE-bench Verified tasks.
The result is useful evidence for these exact cells, not a population estimate.

## Result

Forge pinned beats the matching native harness in aggregate for both provider
families:

| Same-model comparison | Official resolves | Wall time | Raw tokens | Cache-adjusted tokens |
|---|---:|---:|---:|---:|
| Forge pinned to GPT-5.6 | **3 / 6** | **1,049.335s** | **2,315,497** | **1,266,409.00** |
| Native Codex CLI | 0 / 6 | 1,395.112s | 5,216,740 | 1,623,268.00 |
| Forge change | **+3 solves** | **24.78% faster** | **55.61% lower** | **21.98% lower** |
| Forge pinned to Claude 5 | **2 / 4** | **650.260s** | **4,456,211** | **1,291,738.25** |
| Native Claude Code | 1 / 4 | 781.563s | 8,168,748 | 2,260,959.00 |
| Forge change | **+1 solve** | **16.80% faster** | **45.45% lower** | **42.87% lower** |

Regular full-mesh auto also meets the target:

| Mesh auto vs. average native model pair | Official resolves | Wall time | Raw tokens | Cache-adjusted tokens |
|---|---:|---:|---:|---:|
| Forge mesh auto, one run per task | **1 / 2** | **242.643s** | **817,733** | **339,269.00** |
| Average of five native model pairs | 0.2 / 2 | 435.335s | 2,677,097.60 | 776,845.40 |
| Forge change | Equal to or better than every pair | **44.26% faster** | **69.45% lower** | **56.33% lower** |

“0.2 / 2” is the arithmetic average across five native model pairs, not a
fractional solve. In concrete terms, native Opus resolved 1/2 and the other
four model pairs resolved 0/2; mesh resolved 1/2.

## Full-mesh auto against every native model

Mesh ran each unique task once and reused that result against all corresponding
native cells. It was not rerun per comparator.

| Native comparator | Native resolves | Native wall | Mesh faster | Native raw tokens | Mesh raw reduction | Native cache-adjusted | Mesh adjusted reduction |
|---|---:|---:|---:|---:|---:|---:|---:|
| Claude Opus 5 | **1 / 2** | 291.594s | **16.79%** | 1,825,396 | **55.20%** | 546,576.25 | **37.93%** |
| Claude Sonnet 5 | 0 / 2 | 489.969s | **50.48%** | 6,343,352 | **87.11%** | 1,714,382.75 | **80.21%** |
| GPT-5.6 Luna | 0 / 2 | 457.939s | **47.01%** | 2,213,705 | **63.06%** | 663,689.00 | **48.88%** |
| GPT-5.6 Sol | 0 / 2 | 652.792s | **62.83%** | 2,160,476 | **62.15%** | 657,884.00 | **48.43%** |
| GPT-5.6 Terra | 0 / 2 | 284.381s | **14.68%** | 842,559 | **2.95%** | **301,695.00** | **−12.45%** |

The Terra row is the important exception to the aggregate efficiency claim:
mesh used 12.45% more cache-adjusted tokens than native Terra on these two
tasks, although it used 2.95% fewer raw tokens, was 14.68% faster, and solved
one additional task. The “substantially more token-efficient” conclusion is
supported across the five-comparator average and four of five individual model
pairs, not against Terra’s cache-adjusted total.

### Mesh cells

| Task | Routed model | Official | Wall | Raw tokens | Cache-adjusted | Integrity |
|---|---|---:|---:|---:|---:|---|
| `django__django-14376` | `codex-oauth::gpt-5.6-terra` | resolved | 72.701s | 235,962 | 92,730.00 | clean |
| `matplotlib__matplotlib-25960` | `codex-oauth::gpt-5.6-terra` | unresolved | 169.942s | 581,771 | 246,539.00 | clean |

Both traces include the routing event and all parent/child session usage. The
small route-classification call is included in the totals rather than removed.

## Pinned per-cell result

### GPT-5.6 through Forge vs. native Codex CLI

| Model | Task | Forge / native official | Forge / native wall | Forge / native raw tokens | Forge / native cache-adjusted |
|---|---|---:|---:|---:|---:|
| Terra | Matplotlib | 0 / 0 | 222.888s / 214.782s | 520,973 / 579,443 | 251,405 / 210,995 |
| Luna | Matplotlib | 0 / 0 | 216.320s / 366.245s | 694,445 / 1,890,684 | 430,637 / 552,060 |
| Sol | Matplotlib | 0 / 0 | 216.265s / 510.853s | 288,210 / 1,778,025 | 160,338 / 526,377 |
| Luna | Django | **1 / 0** | 126.402s / 91.694s | 322,670 / 323,021 | 202,862 / 111,629 |
| Sol | Django | **1 / 0** | 145.225s / 141.939s | 258,004 / 382,451 | 123,988 / 131,507 |
| Terra | Django | **1 / 0** | 122.235s / 69.599s | 231,195 / 263,116 | 97,179 / 90,700 |

### Claude 5 through Forge vs. native Claude Code

| Model | Task | Forge / native official | Forge / native wall | Forge / native raw tokens | Forge / native cache-adjusted |
|---|---|---:|---:|---:|---:|
| Opus 5 | Matplotlib | **1 / 0** | 347.653s / 253.700s | 1,975,035 / 1,477,772 | 579,972 / 433,842.50 |
| Sonnet 5 | Matplotlib | 0 / 0 | **203.889s / 456.885s** | **1,943,608 / 5,941,908** | **545,728 / 1,589,898** |
| Opus 5 | Django | **1 / 1** | 66.547s / 37.894s | **289,230 / 347,624** | **88,770 / 112,733.75** |
| Sonnet 5 | Django | 0 / 0 | **32.171s / 33.084s** | **248,338 / 401,444** | **77,268.25 / 124,484.75** |

These tables show why aggregate claims should not be read as “Forge wins every
cell.” Forge has startup/orchestration overhead on several short cells, and the
quality gain on Opus Matplotlib required more work than the unsuccessful native
attempt.

## Controlled method

- Dataset: `swe-verified-easy-medium2.jsonl`, SHA-256
  `48320e25bd9bedc03d6566b08558e438b12efdecdad87052cd29c96401ad5c3e`.
- Tasks and official bases:
  - `django__django-14376`,
    `d06c5b358149c02a62da8a5469264d05f29ac659`
  - `matplotlib__matplotlib-25960`,
    `1d0d255b79e84dfc9f2123c5eb85a842d342f72b`
- Native Codex: `codex-cli 0.145.0`; GPT-5.6 Sol, Terra, and Luna;
  regular `high`.
- Native Claude Code: `2.1.220`; resolved identifiers
  `claude-opus-5[1m]` and `claude-sonnet-5`; regular `high`.
- Forge: `2.11.0`. The current Claude and mesh binary SHA-256 is
  `587cafd08a6c4f1ce7a15adcaa35b5881b310518b572341b989815fb3361741d`.
- Timeout: 1,500 seconds for every cell.
- Quality: official `swebench==4.1.0` Docker evaluator only. A successful
  process, non-empty patch, or model claim never counts as a resolve.
- Raw tokens: input plus output, with cached input already included in input.
- Cache-adjusted sensitivity: uncached input + 0.25 × cached input + output.
  This is not a reconstruction of either provider’s proprietary quota formula.
- Each cell used the exact official base tree represented by one synthetic,
  reachable Git commit. The preparation removed remotes and upstream objects.
- The integrity preamble prohibited network, remote repositories, issue
  trackers, pull requests, external search, and later Git history.
- Patch capture diffed against the synthetic base, preserving both committed
  and uncommitted agent changes.
- Each Forge cell used a separate Store database. Mesh also used a separate
  repository root per task.
- Every trace was audited. Commands that access Git history/remotes, network
  commands, and external search tools invalidate a cell.
- Mesh used the regular user configuration with `auto_discover`,
  `auto_orchestrate`, and failover enabled. Its command included no `--model`
  and no effort override.

The full [cell-validity ledger](cell-validity-2026-07.md) records all 83 formal
cells considered: 27 methodologically valid cells and 56 invalid or superseded
cells. The selected headline uses the sole clean native cells, the earliest
valid Codex replacements, Claude cells from the current bridge binary, and the
single clean mesh run per task. Later unchanged duplicate Codex cells are not
cherry-picked.

## Quota accounting

The original refreshed weekly baselines for this goal were 26% Codex and 23%
Claude. Hard cumulative stops were therefore 36% and 28%.

Final refreshed utilization was 33% Codex and 26% Claude: conservative
account-wide deltas of +7 and +3 percentage points, inside the +10/+5 limits.
Helm was refreshed before the clean mesh study and after every provider arm.
Only one paid arm ran at a time. The two final mesh arms did not move the
integer percentages.

Per selected cell, the refreshed integer-percent observations were:

| Provider arm / model / task | Weekly utilization before → next/post refresh | Observed Δ |
|---|---:|---:|
| Native Codex / Terra / Matplotlib | 30% → 30% | +0 pp |
| Native Codex / Luna / Matplotlib | 30% → 30% | +0 pp |
| Native Codex / Sol / Matplotlib | 30% → 30% | +0 pp |
| Native Codex / Luna / Django | 30% → 30% | +0 pp |
| Native Codex / Sol / Django | 30% → 30% | +0 pp |
| Native Codex / Terra / Django | 30% → 30% | +0 pp |
| Forge pinned / Terra / Matplotlib | 30% → 31% | +1 pp |
| Forge pinned / Luna / Matplotlib | 31% → 31% | +0 pp |
| Forge pinned / Sol / Matplotlib | 31% → 31% | +0 pp |
| Forge pinned / Luna / Django | 31% → 31% | +0 pp |
| Forge pinned / Sol / Django | 31% → 31% | +0 pp |
| Forge pinned / Terra / Django | 31% → 31% | +0 pp |
| Native Claude / Opus / Matplotlib | 24% → 24% | +0 pp |
| Native Claude / Sonnet / Matplotlib | 24% → 25% | +1 pp |
| Native Claude / Opus / Django | 25% → 25% | +0 pp |
| Native Claude / Sonnet / Django | 25% → 25% | +0 pp |
| Forge pinned / Sonnet / Matplotlib | 26% → 26% | +0 pp |
| Forge pinned / Opus / Matplotlib | 26% → 26% | +0 pp |
| Forge pinned / Opus / Django | 26% → 26% | +0 pp |
| Forge pinned / Sonnet / Django | 26% → 26% | +0 pp |
| Mesh auto / Terra route / Django | 33% → 33% | +0 pp |
| Mesh auto / Terra route / Matplotlib | 33% → 33% | +0 pp |

These are coarse account-wide bucket observations, not causal per-request
billing. A `+0 pp` cell still consumed tokens but did not cross an integer
boundary. The aggregate +7/+3 deltas include invalid or superseded arms and
any other account activity, so they are the conservative numbers used for cap
enforcement.

## Superseded results and limits

The earlier GPT-5.6 headline suite is invalid for the current comparison where
traces prove access to non-isolated Git history or external solution sources;
it also used `xhigh` instead of the requested regular `high`. Earlier Claude
and mesh headline cells used non-isolated repositories. They remain preserved
in the ledger but are excluded from the current headline.

The old 6/6-to-0/6 Codex change is not attributed solely to contamination. The
clean replacement also changed `xhigh` to `high`, added the integrity preamble,
isolated repository history, used later harness code, and is stochastic. Only
a controlled study could estimate the size of each effect.

This study has two unique tasks, one attempt per cell, and no confidence
interval worth interpreting. It establishes the requested target on the tested
cells, not a universal ranking. In particular:

- Forge is not faster or lower-token in every pinned cell.
- Mesh is not cache-adjusted-token-better than native Terra on this sample.
- Matplotlib remained unresolved for clean mesh.
- More tasks and repeated seeds would be needed for a general performance
  claim, but were intentionally not purchased under the quota constraint.
