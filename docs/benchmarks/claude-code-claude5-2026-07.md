# Forge vs. native Claude Code on Claude 5

**Run date:** 2026-07-27

**Models:** `claude-opus-5[1m]` and `claude-sonnet-5`

**Effort:** `high` (not `xhigh`)

**Evaluator:** official Docker harness from `swebench==4.1.0`

## Result

Forge beat native Claude Code on official quality: **5/6 resolved versus
4/6**. On the five pairs with the same official outcome, Forge was **19.72%
faster**, used **38.14% fewer processed tokens**, and used **36.74% fewer
cache-adjusted tokens**.

| Six matched SWE-bench Verified pairs | Forge | Native Claude Code | Result |
|---|---:|---:|---|
| Official Docker resolves | **5 / 6** | 4 / 6 | **Forge +1 solve** |
| Evaluator errors | 0 | 0 | Complete |
| Quality-matched wall time (5 pairs) | **1,242.588s** | 1,547.852s | **Forge 19.72% faster** |
| Quality-matched processed tokens | **7,175,961** | 11,600,158 | **Forge 38.14% lower** |
| Quality-matched cache-adjusted tokens | **2,010,283.50** | 3,177,565.00 | **Forge 36.74% lower** |
| Both-resolved wall time (4 pairs) | **1,220.719s** | 1,532.995s | **Forge 20.37% faster** |
| Both-resolved processed tokens | **6,991,619** | 11,442,964 | **Forge 38.90% lower** |

“Quality-matched” means both arms received the same official outcome, whether
resolved or unresolved. “Both-resolved” is the stricter subset where both
patches passed. These are the meaningful speed and efficiency comparisons:
crediting an eight-second empty response as a fast, low-token success would be
misleading.

For completeness, the unconditional six-pair totals were 1,653.716 seconds and
12,147,601 tokens for Forge versus 1,555.938 seconds and 11,651,172 tokens for
native Claude Code. Forge is 6.28% slower and uses 4.26% more tokens under that
accounting because native Sonnet returned an empty patch after 8.086 seconds on
the hard task that Forge solved. The result is preserved, but it is not used as
an efficiency win for native Claude Code.

## Per-model result

| Model | Official resolves | Wall time on quality-matched pairs | Processed tokens on quality-matched pairs | Cache-adjusted tokens |
|---|---:|---:|---:|---:|
| Opus 5 (all 3 pairs matched) | 3/3 vs. 3/3 | **897.082s vs. 1,138.804s (21.23% faster)** | **3,938,659 vs. 6,342,195 (37.90% lower)** | **35.76% lower** |
| Sonnet 5 (2 matched pairs) | **2/3 vs. 1/3** overall | **345.506s vs. 409.048s (15.53% faster)** | **3,237,302 vs. 5,257,963 (38.43% lower)** | **37.93% lower** |

Opus provides the cleanest efficiency comparison: both harnesses resolve every
task, while Forge is faster and uses fewer tokens in aggregate. Sonnet has one
discordant pair—Forge solved it and native Claude Code did not—so the efficiency
table excludes that pair while the official quality score includes it.

## Per-pair results

| Model | Difficulty | Instance | Forge | Native | Forge wall | Native wall | Forge tokens | Native tokens |
|---|---|---|---:|---:|---:|---:|---:|---:|
| Opus 5 | `<15 min` | `django__django-14376` | Resolved | Resolved | 69.447s | **54.603s** | **258,835** | 518,625 |
| Opus 5 | `15 min–1 hour` | `matplotlib__matplotlib-25960` | Resolved | Resolved | **492.835s** | 539.326s | **1,922,470** | 2,947,123 |
| Opus 5 | `1–4 hours` | `scikit-learn__scikit-learn-25102` | Resolved | Resolved | **334.800s** | 544.875s | **1,757,354** | 2,876,447 |
| Sonnet 5 | `<15 min` | `django__django-14376` | Unresolved | Unresolved | 21.869s | **14.857s** | 184,342 | **157,194** |
| Sonnet 5 | `15 min–1 hour` | `matplotlib__matplotlib-25960` | Resolved | Resolved | **323.637s** | 394.191s | **3,052,960** | 5,100,769 |
| Sonnet 5 | `1–4 hours` | `scikit-learn__scikit-learn-25102` | **Resolved** | Empty patch | 411.128s | 8.086s | 4,971,640 | 51,014 |

All twelve submitted arms had complete wall and token telemetry. All non-empty
predictions were evaluated in Docker. The native Sonnet hard arm's literal final
answer was:

> What want do? Implement dtype-preserve feature, or just discuss approach?

It made no repository change, and the official evaluator classified it as an
empty patch.

## Token accounting

Claude reports uncached input, cache reads, and cache creation as additive
counters. The primary processed-token total is:

```text
input + cache_read_input + cache_creation_input + output
```

The separate sensitivity measure weights cache-read input at 0.25 while counting
uncached/cache-created input and output at full value. It is included because a
cached token is cheaper than a fresh token, but it is not presented as
Anthropic's proprietary subscription-quota formula.

On the five quality-matched pairs, Forge had lower processed tokens in 4/5
pairs (`p=0.375`, two-sided exact sign test) and lower cache-adjusted tokens in
5/5 (`p=0.0625`). The direction is encouraging, but the sample is intentionally
small and is not enough for a high-certainty population claim.

## Protocol

The comparison held constant:

- the exact SWE-bench issue prompt, repository, and base commit;
- Claude Code 2.1.220 on the same host and subscription;
- regular `high` effort for both harnesses;
- authoritative non-billing Claude initialization, which resolved
  `sonnet` to `claude-sonnet-5` and `opus[1m]` to
  `claude-opus-5[1m]`;
- fresh isolated Git worktrees per arm;
- a 1,500-second generation timeout;
- the same network and repository-history visibility;
- official `swebench==4.1.0` Docker evaluation.

The Forge arm used:

```bash
forge run \
  --mode bypass \
  --output-format stream-json \
  --model claude-cli::<model> \
  "<issue prompt>"
```

The native arm used:

```bash
claude -p \
  --output-format stream-json \
  --verbose \
  --include-partial-messages \
  --dangerously-skip-permissions \
  --effort high \
  --model <model> \
  "<issue prompt>"
```

Forge set `FORGE_MESH__DEFAULT_EFFORT=high`, so both paths used the same effort.
The raw arm retained the user's installed native Claude Code environment; the
study measures the harnesses users actually run rather than an invented
configuration with capabilities stripped away.

## Sample design and quota

The final study contains two models across three published difficulty bands,
for six matched model-task pairs and twelve generation arms. This is one third
of the 18-pair Codex study, proportional to the user's Claude allowance:

- refreshed weekly baseline: 21%;
- absolute user cap: 31% (`+10` percentage points);
- runner hard stop: 30%, leaving a one-point safety margin;
- final forced Helm reading: 22%.

The study therefore consumed only one observed weekly percentage point. Claude's
in-band rate-limit event exposed the five-hour bucket but not weekly use. The
runner failed closed after every arm, Helm usage was forcibly refreshed, and
only the exact counterpart or next predeclared arm was resumed.

The post-fix hard task was completed first. The final six-pair design and the
easy/medium dataset were then fixed before the remaining four pairs. The
easy/medium dataset hash is:

```text
48320e25bd9bedc03d6566b08558e438b12efdecdad87052cd29c96401ad5c3e
```

## Runtime identity and artifacts

The measured binary and source identity were:

```text
Forge commit:       11f6aa92b46c86a30f047f2f3146bed4003bccd0
Forge binary SHA:   228b5e4323e67e3dbdcb66a1c0401210a638d14203c6b0bc869fba47a53a792d
Claude Code:        2.1.220
Effort:             high
Evaluator:          swebench==4.1.0
```

Raw events, stderr, patches, manifests, quota checks, trial summaries,
predictions, and official evaluator reports are retained on the benchmark host
under:

```text
~/.local/share/forge/harness-bench-20260727/
  swe-verified-stratified6-claude5-high-v2/
  swe-verified-scikit25102-claude-opus5-high-v1/
  swe-verified-easy-medium2-claude5-high-v1/
```

Rebuild the combined report with:

```bash
PYTHONPATH=scripts/harness-bench \
python3 scripts/harness-bench/analyze_claude_swe.py \
  --run-dir /path/to/sonnet-hard-run \
  --run-dir /path/to/opus-hard-run \
  --run-dir /path/to/easy-medium-run \
  --out-json /path/to/official-analysis.json \
  --out-markdown /path/to/official-analysis.md
```

The analyzer deliberately ignores `submitted_ids` as an outcome because
SWE-bench includes every prediction-file row there even when `--instance_ids`
evaluates only one. It binds only explicit `resolved_ids`, `unresolved_ids`, and
`empty_patch_ids`, and fails on missing/duplicate outcomes or evaluator errors.

## Important caveats

- **Small sample:** `n=3` per model and `n=6` overall. The official quality
  difference is one discordant pair; exact paired McNemar/sign `p=1.0`.
- **Future-history visibility:** native Opus searched visible repository history
  on the hard scikit-learn task and found the later upstream issue commit. Both
  arms had the same history and network access, so the comparison remains
  symmetric, but this can make the task easier than a sealed historical run.
- **Fast failures distort efficiency:** unconditional time/token totals include
  the native Sonnet empty patch. That is why quality-matched and both-resolved
  subsets are reported prominently and unconditional totals are still disclosed.
- **One host and subscription:** service load, cache state, and local system
  conditions may differ elsewhere. Pair order was counterbalanced where
  possible, but cannot eliminate all temporal variance.
- **No claim beyond these models/tasks:** the bridge improvements are
  model-agnostic where they touch transport, MCP readiness, tool aliasing,
  no-replay safety, and watchdog behavior, but performance must be remeasured for
  every model/workload family.

## Exclusions

The old Sonnet benchmark worktree came from a much older Forge version and was
not used. A failed post-fix calibration run at
`swe-verified-stratified6-claude5-high-v1` hit the old 120-second stream watchdog;
it is retained as a diagnostic artifact but excluded from every official result
above. Before official timed runs, an unrelated stale scratch benchmark process
group containing 24 CPU spin loops was terminated. No official arm ran under
that contamination.
