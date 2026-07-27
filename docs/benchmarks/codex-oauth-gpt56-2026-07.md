# Forge vs native Codex CLI on GPT-5.6

Status: complete — optimized matched rerun passed
Study date: 2026-07-26–27
Current branch: `bench/codex-oauth-gpt56-20260726`

## Executive summary

The current optimized Forge build matches native raw Codex on official quality and beats it
on both elapsed time and whole-session token use on the fresh, predeclared matched run:

| Metric | Forge | Native raw Codex | Result |
|---|---:|---:|---|
| Official Docker resolves | 18/18 | 18/18 | Quality parity |
| Evaluator errors | 0 | 0 | Complete |
| Generation wall time | 3,299.841s | 5,574.566s | Forge 40.81% faster |
| Whole-session tokens | 9,692,604 | 24,643,373 | Forge 60.67% lower |
| Tokens per resolve | 538,478.00 | 1,369,076.28 | Forge 60.67% lower |
| Faster matched pairs | 13/18 | 5/18 | Forge median reduction 40.19% |
| Lower-token matched pairs | 17/18 | 1/18 | Exact sign-test `p=0.00014496` |

All three models independently resolved 6/6 tasks for both harnesses. Forge's advantage was
also consistent by model:

| Model | Forge wall | Raw wall | Forge wall reduction | Forge tokens | Raw tokens | Forge token reduction |
|---|---:|---:|---:|---:|---:|---:|
| GPT-5.6 Luna | 1,116.496s | 1,668.046s | 33.07% | 3,763,504 | 10,387,906 | 63.77% |
| GPT-5.6 Sol | 1,258.841s | 2,106.314s | 40.23% | 2,693,232 | 7,324,162 | 63.23% |
| GPT-5.6 Terra | 924.504s | 1,800.206s | 48.64% | 3,235,868 | 6,931,305 | 53.32% |

The matched analyzer reports a weighted 40.81% wall-time reduction and 60.67% token
reduction. Its paired median reductions are 40.19% for wall time and 57.06% for tokens;
the seeded bootstrap 95% interval for median token reduction is 42.95%–67.29%.

The run used the same account, GPT-5.6 Sol/Terra/Luna models, `xhigh` reasoning, official
commits, prompts, 1,500-second generation timeout, seed `20260726`, and native raw-Codex
profile. It completed 36/36 arms with 36 non-empty patches, zero agent timeouts, 18 Forge
root sessions, and zero child or fork sessions. Two Forge processes ended after provider
availability failures, but retained their patches; official Docker evaluation resolved
both. Weekly usage finished at 25%, below the absolute 30% stop.

Authoritative artifact root:

```text
/home/floris/.local/share/forge/harness-bench-20260726/swe-verified-stratified6-gpt56-native-v2-direct-env-cap/
```

The suite manifest pins Forge commit `f816368d1e50dcfdaf919bcc747b193fb05eb899`,
binary SHA-256 `1039d36f31b39c8b7454a0061dc0a392bdd6161c9f9242a1d54b06b3796f35df`,
and dataset SHA-256 `3624682a509d9c9b75723a35d54e211ed62c99c5897a96de36f5a2118c10da61`.
The six official `swebench==4.1.0` reports are under `evaluation/`.

The stale Claude/Sonnet worktree, its patches, results, and baselines were not used.

### Final verification of the optimized rerun

- Exactly six unique official reports were present: Forge and raw Codex for Sol, Terra,
  and Luna. Every report contained six submitted, six completed, six resolved, zero
  unresolved, zero empty patches, and zero evaluator errors.
- `analyze_codex_oauth_swe.py` bound all 18 matched pairs and reproduced the aggregate
  quality, wall-time, and whole-session token totals above.
- `cargo fmt --all -- --check` passed.
- `cargo clippy --locked --all-targets --all-features -- -D warnings` passed.
- `cargo test --locked --all --all-features` passed.
- `cargo build --release --locked --bin forge` passed.
- Ruff and Python byte-compilation passed for `scripts/harness-bench`.
- Benchmark-script unit discovery passed all four tests.

The first full test attempt encountered a shared `/tmp` user-quota failure caused by
multi-day stale temporary state, not a Forge assertion. After removing only a rebuildable
2.23 GB test fixture and terminating 48 orphaned CPU stress processes from the stale Claude
worktree, the exact failed test and the complete workspace suite passed. No repository,
benchmark report, prediction, evaluator log, or SWE-bench artifact was removed.

## Historical pre-optimization result

This study compares Forge's own coding harness with the user's installed native Codex CLI while
holding the authenticated OpenAI account, model, reasoning effort, task, starting repository, and
timeout constant.

The completed study supports a narrower, split conclusion:

1. **Controlled-fixture ceiling:** both harnesses passed all 12 post-fix matched task pairs. With
   recursively spawned child sessions included, Forge used 73.0% fewer tokens and 43.75% less
   wall time on those deterministic fixtures.
2. **Official quality point estimate favors raw Codex:** on the predeclared six-instance
   SWE-bench Verified subset across three models, Forge resolved 16/18 model-task pairs and raw
   Codex resolved 18/18. Both discordances were raw-only Django solves. With only two discordances,
   the exact two-sided paired p-value is 0.5, so the subset is too small to establish a reliable
   quality difference.
3. **Hard-task orchestration is inefficient:** after correcting the harness to include recursive
   child-session usage, Forge used 20.11% more tokens in aggregate on the official phase and was
   lower-token in only 7/18 pairs (`p=0.48068237`). Forge also took 58.64% more wall time.

The evidence therefore does **not** support a claim that Forge has better general coding
intelligence, a higher SWE-bench resolve rate, or a universal speed/token advantage. It does show a
large and repeatable efficiency advantage on the controlled fixtures. On this small official
subset, recursive subagent fan-out instead made Forge both slower and higher-token in aggregate.

The historical Claude Sonnet benchmark worktree is stale, was built against a much older Forge,
and is excluded from this study. None of its results are used below.

## What was compared

| Dimension | Forge arm | Native arm |
|---|---|---|
| Harness | Forge headless coding loop | Installed `codex exec` CLI |
| Provider/model | `codex-oauth::<model>` | `-m <model>` |
| Models | GPT-5.6 Sol, Terra, Luna | GPT-5.6 Sol, Terra, Luna |
| Reasoning | `xhigh` | `model_reasoning_effort="xhigh"` |
| Account | Forge keyring OAuth | Codex CLI OAuth |
| Permission mode | Bypass in disposable repo | Bypass approvals/sandbox in disposable repo |
| Correctness | Independent deterministic verifier | Same verifier |
| Repeats | One per model/scenario pair | One per model/scenario pair |
| Raw CLI profile | n/a | User's native installed CLI configuration |

The Forge and native Codex credentials were compared by hashing their account identifiers locally;
the hashes matched. No credential or account identifier was copied into benchmark artifacts.

`native` is an important qualifier. The installed Codex CLI loaded its normal built-in/user prompt,
rules, Graphify/Debugging skills, and tool surface. This is the user's real harness, but another
Codex installation with a different skill catalog may have different overhead.

## Controlled protocol

The committed runner is
[`scripts/harness-bench/compare_codex_oauth.py`](../../scripts/harness-bench/compare_codex_oauth.py).

For every matched pair it:

1. creates a fresh Git repository from the same fixture;
2. passes the same user prompt to both arms;
3. pins the same GPT-5.6 model and `xhigh` effort;
4. alternates which arm runs first after a seeded pair shuffle;
5. applies the same per-trial model and verifier timeouts;
6. captures raw JSONL, stderr, patch, process timing, verifier logs, session id, and quota data;
7. treats process success as insufficient—the independent verifier must pass; and
8. stops before another provider call when fresh quota telemetry is missing or reaches the cap.

The scenarios are the current manual end-to-end fixtures:

| Scenario | Independent contract |
|---|---|
| Multifile reservations | Python unit-test suite |
| Ordered Go pipeline | `gofmt`, `go vet`, and race-enabled tests |
| TypeScript config recovery | `npm test` and a fresh strict no-emit TypeScript build |
| Rust transaction ledger | formatting, Clippy with warnings denied, and all-target tests |

The TypeScript verifier originally invoked a reference-specific `npm run lint` alias. The task
contract requires a strict build, not that alias. The live reports were preserved unchanged and a
separate rescoring tool produced corrected, auditable reports:

```bash
scripts/harness-bench/rescore_codex_oauth.py --run-dir /path/to/run
```

## Token accounting

`input_tokens` already includes cached input in current Forge Responses telemetry and Codex CLI
0.145.0. `cached_input_tokens` is therefore a subset and is never added a second time.

Primary model work is:

```text
total_tokens = input_tokens + output_tokens
```

The report also retains a labelled sensitivity metric that weights cached input at 0.25. It is not
presented as OpenAI's private weekly-subscription formula.

Forge totals come from the complete SQLite usage ledger. This includes post-turn auxiliary calls
such as memory extraction. The stream counter is retained separately so discrepancies are visible.

## Calibration

Calibration used a trivial Luna prompt to confirm that the two harness surfaces carry very
different fixed overhead:

| Arm | Input | Output |
|---|---:|---:|
| Forge/Luna | 830 | 8 |
| Raw Codex/Luna | 16,197 | 22 |

Calibration is prompt-surface evidence only. It is not task-quality evidence and is excluded from
the matched task results.

## Corrected controlled baseline

Artifact root:

```text
/home/floris/.local/share/forge/harness-bench-20260726/baseline-current-native-v1/
```

Authoritative reports:

```text
aggregate.corrected.json
aggregate.corrected.md
trials/*/summary.corrected.json
```

### Overall

| Metric | Forge | Native Codex | Result |
|---|---:|---:|---|
| Contract passes | 12/12 | 12/12 | Quality parity / ceiling |
| Total tokens | 2,742,636 | 8,298,661 | Forge 66.95% lower |
| Total wall time | 2,391.720s | 3,784.083s | Forge 36.8% lower |
| Lower-token pairs | 12/12 | 0/12 | Sign test `p=0.00048828` |
| Faster pairs | 11/12 | 1/12 | Sign test `p=0.00634766` |

Median paired token reduction was 69.2%. A seeded 20,000-sample bootstrap gave a 95% interval of
48.99%–74.9% for that median.

### By model

| Model | Forge passes | Raw passes | Forge tokens | Raw tokens | Forge reduction |
|---|---:|---:|---:|---:|---:|
| GPT-5.6 Luna | 4/4 | 4/4 | 954,342 | 2,740,572 | 65.18% |
| GPT-5.6 Sol | 4/4 | 4/4 | 992,648 | 2,578,403 | 61.50% |
| GPT-5.6 Terra | 4/4 | 4/4 | 795,646 | 2,979,686 | 73.30% |

### By scenario

| Scenario | Forge passes | Raw passes | Forge tokens | Raw tokens | Forge reduction |
|---|---:|---:|---:|---:|---:|
| Ordered Go pipeline | 3/3 | 3/3 | 542,498 | 1,990,784 | 72.75% |
| Multifile reservations | 3/3 | 3/3 | 647,706 | 2,721,171 | 76.20% |
| Rust transaction ledger | 3/3 | 3/3 | 573,171 | 1,850,648 | 69.03% |
| TypeScript config recovery | 3/3 | 3/3 | 979,261 | 1,736,058 | 43.59% |

## Findings and improvements

### 1. Autofix auto-detection silently enabled a default-off feature

Observed behavior:

- `auto_detect=true` filled `lint_cmd` and set `auto_lint=true` even though autofix is documented
  and configured off by default.
- Autofix ran redundant checks in 8 of 12 Forge baseline trials.
- Luna and Sol TypeScript trials tried a nonexistent `npm run lint`, injected the failure into the
  conversation, and spent a full extra model loop changing an unrequested package-script alias.
- Those trials made 30 and 31 provider calls; the Terra TypeScript arm made 14.

Fix:

- Detection now fills only commands the user explicitly enabled.
- Detection never changes `auto_lint` or `auto_test` from false to true.
- A regression test proves default-off remains off.

### 2. NPM detection assumed a `lint` script existed

Observed behavior:

- Any `package.json` selected `npm run lint`, whether or not the script existed.

Fix:

- Forge parses `package.json` and selects the first existing script among `lint`, `typecheck`, and
  `check`.
- `npm test` is selected only when a `test` script exists.
- Missing scripts remain empty and do not activate a check.

### 3. The SWE-bench external Codex parser double-counted cached input

Observed behavior:

- Codex 0.145.0 reports cached/write input as subsets of `input_tokens`.
- `parse_external_usage` added those counters again.

Fix:

- Cache semantics are provider-specific: Claude's separate counters remain additive; Codex cache
  counters remain subsets.
- A regression fixture with input 16,646, cached 9,984, and write 512 now reports 16,646—not
  27,142.

### 4. Fully-qualified Codex pins made an unnecessary pre-turn quota probe

Observed behavior:

- A fully-qualified `codex-oauth::<model>` pin bypasses mesh selection.
- The task response itself supplies fresh quota headers.
- The pre-turn Luna probe could not change routing and added subscription usage plus latency.

Fix:

- Any fully-qualified pin skips the routing-pressure refresh.
- Unpinned/bare model ids retain the refresh because they may mesh-route through Codex.

### 5. Terminal usage preceded awaited auxiliary memory work

Observed behavior:

- Headless Forge awaited post-turn memory persistence but emitted final usage first.
- Baseline undercounts were 353–548 tokens per Forge trial.
- Synthetic side-call messages are inactive by design, so the active-transcript token API could
  never provide complete consumed usage.

Fix:

- Store now exposes a distinct cumulative provider-consumption ledger that includes auxiliary and
  later-deactivated calls. Undo still removes calls from the active transcript counter, but cannot
  pretend their provider quota was refunded.
- Headless mode awaits auxiliary work, emits cumulative `Cost`, then keeps `Done` last.
- Interactive channel-backed surfaces keep early completion and detach auxiliary work.
- Tests cover input, cached-input subset, output, memory inclusion, active-vs-consumed semantics,
  and `Cost`/`Done` ordering.

### 6. `bench swe` used the active counter after terminal accounting was fixed

Observed behavior:

- `session_usage_db()` still called the active-transcript token API.
- The built-in SWE-bench sidecar could therefore omit auxiliary usage.

Fix:

- `session_usage_db()` now reads the cumulative provider-consumption ledger.
- The terminal accounting regression also asserts this public benchmark-facing API.

## Targeted post-fix confirmation

Artifact root:

```text
/home/floris/.local/share/forge/harness-bench-20260726/postfix-typescript-luna-sol-v2.10.2/
```

The targeted rerun covered the two model/scenario combinations that had triggered the extra
autofix loop.

| Model/arm | Pass | Calls | Tokens | Wall time |
|---|---:|---:|---:|---:|
| Forge/Luna | yes | 28 | 214,493 | 202.794s |
| Raw/Luna | yes | n/a | 853,318 | 382.761s |
| Forge/Sol | yes | 27 | 255,014 | 237.090s |
| Raw/Sol | yes | n/a | 660,706 | 495.809s |

Matched Forge reductions were 74.9% for Luna and 61.4% for Sol; median 68.1%. Weighted wall-time
reduction across the two pairs was 49.93%.

The targeted pre/post Forge comparison:

| Model | Calls before → after | Tokens before → after | Wall before → after |
|---|---:|---:|---:|
| Luna | 38 → 28 | 406,142 → 214,493 (−47.2%) | 281.740s → 202.794s (−28.0%) |
| Sol | 31 → 27 | 388,378 → 255,014 (−34.3%) | 228.373s → 237.090s (+3.8%) |

The Sol wall time did not improve in this single stochastic rerun despite fewer calls. The report
does not convert the token/call improvement into an unsupported latency claim.

No post-fix runtime artifact contains an autofix warning or a nonexistent-lint attempt. Parent
terminal streams remain complete for their parent sessions; the calls and tokens above also include
their child-session trees.

## Full post-fix controlled rerun

Artifact root:

```text
/home/floris/.local/share/forge/harness-bench-20260726/postfix-full-v2.10.2-native-v1/
```

All 24 planned trials completed without a stop condition. The corrected report is
`aggregate.corrected.{json,md}`.

### Overall

| Metric | Forge | Native Codex | Result |
|---|---:|---:|---|
| Contract passes | 12/12 | 12/12 | Quality parity / ceiling |
| Total tokens | 2,718,582 | 10,067,973 | Forge 73.0% lower |
| Total wall time | 2,262.804s | 4,022.659s | Forge 43.75% lower |
| Lower-token pairs | 12/12 | 0/12 | Sign test `p=0.00048828` |
| Faster pairs | 11/12 | 1/12 | Sign test `p=0.00634766` |

Median paired token reduction was 64.5%; the seeded bootstrap 95% interval was 45.08%–83.21%.

### By model

| Model | Forge passes | Raw passes | Forge tokens | Raw tokens | Forge reduction |
|---|---:|---:|---:|---:|---:|
| GPT-5.6 Luna | 4/4 | 4/4 | 804,092 | 4,705,031 | 82.91% |
| GPT-5.6 Sol | 4/4 | 4/4 | 944,881 | 2,976,855 | 68.26% |
| GPT-5.6 Terra | 4/4 | 4/4 | 969,609 | 2,386,087 | 59.36% |

### By scenario

| Scenario | Forge passes | Raw passes | Forge tokens | Raw tokens | Forge reduction |
|---|---:|---:|---:|---:|---:|
| Ordered Go pipeline | 3/3 | 3/3 | 678,364 | 2,337,966 | 70.98% |
| Multifile reservations | 3/3 | 3/3 | 750,958 | 2,257,040 | 66.73% |
| Rust transaction ledger | 3/3 | 3/3 | 614,150 | 3,106,497 | 80.23% |
| TypeScript config recovery | 3/3 | 3/3 | 675,110 | 2,366,470 | 71.47% |

All 12 Forge parent terminal streams were complete for their parent sessions. Recursive child
sessions add 295,376 tokens to the whole-harness total above. No retained runtime JSONL/stderr
contains an autofix warning or missing-lint attempt.

### Forge pre/post comparison

Comparing Forge only avoids attributing raw model variance to the harness fix:

| Forge metric | Baseline | Post-fix | Change |
|---|---:|---:|---:|
| Provider calls | 302 | 330 | +9.3% |
| Tokens | 2,742,636 | 2,718,582 | −0.9% |
| Wall time | 2,391.720s | 2,262.804s | −5.4% |

The causal effect is concentrated where the bug was observed:

| TypeScript Forge metric | Baseline | Post-fix | Change |
|---|---:|---:|---:|
| Provider calls | 109 | 83 | −23.9% |
| Tokens | 979,261 | 675,110 | −31.1% |
| Wall time | 764.392s | 556.676s | −27.2% |

Other scenarios moved in both directions, which is expected from one stochastic repeat. The report
does not attribute their variance to the autofix change.

## Official SWE-bench Verified phase

The controlled suite cannot discriminate quality, so official SWE-bench Verified is the separate
hard-task phase. Dataset and selection artifacts are fresh and external to the stale Sonnet
worktree:

```text
/home/floris/.local/share/forge/harness-bench-20260726/swe-bench-verified-4.1.0.jsonl
/home/floris/.local/share/forge/harness-bench-20260726/swe-verified-stratified-6-seed20260726.jsonl
/home/floris/.local/share/forge/harness-bench-20260726/swe-verified-stratified-6-seed20260726.manifest.json
```

The 500-row source SHA-256 is
`7303cc5795e3707162f9b0ffcc5694f3fd67e20bd9d514cfdce63146fdebc196`.

Before any model outcome was observed, the subset was fixed as:

- two instances from each published band: `<15 min fix`, `15 min - 1 hour`, `1-4 hours`;
- no repository repeated; and
- candidates ranked by
  `sha256("20260726:<difficulty>:<instance_id>")`, taking the first eligible two per band.

| Difficulty | Instance | Repository |
|---|---|---|
| `<15 min fix` | `django__django-14376` | `django/django` |
| `<15 min fix` | `sympy__sympy-12096` | `sympy/sympy` |
| `15 min - 1 hour` | `matplotlib__matplotlib-25960` | `matplotlib/matplotlib` |
| `15 min - 1 hour` | `astropy__astropy-14096` | `astropy/astropy` |
| `1-4 hours` | `scikit-learn__scikit-learn-25102` | `scikit-learn/scikit-learn` |
| `1-4 hours` | `pytest-dev__pytest-6197` | `pytest-dev/pytest` |

The three `>4 hours` instances were excluded because a six-task run cannot represent that band
without either one noisy case or a materially larger budget.

The matched official runner is
[`scripts/harness-bench/compare_codex_oauth_swe.py`](../../scripts/harness-bench/compare_codex_oauth_swe.py).
It preserves raw JSONL, stderr, patches, process results, full Forge ledgers, fresh quota telemetry,
and standard prediction files. Resolution is determined only by `swebench==4.1.0` in Docker.

The official environment was first validated by scoring the dataset's gold patches: all 6/6
resolved with zero evaluator errors. The matched generation then completed all 36 planned arms;
every process exited successfully and produced a nonempty patch. "Produced a patch" was never
treated as "resolved."

Primary artifact root:

```text
/home/floris/.local/share/forge/harness-bench-20260726/swe-verified-stratified6-gpt56-native-v1/
```

Authoritative generated analysis:

```text
official-analysis.json
official-analysis.md
official-analysis-session-tree.json
official-analysis-session-tree.md
evaluation-v2/*.json
```

The committed analyzer is
[`scripts/harness-bench/analyze_codex_oauth_swe.py`](../../scripts/harness-bench/analyze_codex_oauth_swe.py).
It requires exactly one official report per arm/model, rejects evaluator errors or incomplete
telemetry, recursively rolls Forge child-session usage into the parent trial, and binds resolution
back to each matched trial before computing aggregates. The `official-analysis-session-tree.*`
files preserve the corrected re-analysis; the original parent-only reports remain for audit.

### Official overall result

| Metric | Forge | Native Codex | Result |
|---|---:|---:|---|
| Official resolves | 16/18 | 18/18 | Raw-only on 2 pairs |
| Evaluator errors | 0 | 0 | Complete |
| Total generation tokens | 27,122,336 | 22,581,417 | Forge 20.11% higher |
| Total tokens / official resolve | 1,695,146 | 1,254,523 | Forge 35.12% higher |
| Total generation wall time | 8,257.542s | 5,205.356s | Forge 58.64% slower |
| Lower-token pairs | 7/18 | 11/18 | Sign test `p=0.48068237` |
| Faster pairs | 7/18 | 11/18 | Sign test `p=0.48068237` |

Median paired Forge token reduction was −18.35%, meaning the median pair used more Forge tokens.
Its seeded bootstrap 95% interval was −69.39%–26.41%, which includes zero. The two quality
discordances both favored raw Codex; the exact two-sided paired McNemar/sign p-value is 0.5.

The "tokens / official resolve" row divides all generation tokens—including unresolved attempts—by
official resolves. It does not erase the cost of Forge's two failures.

### Official result by model

| Model | Forge resolves | Raw resolves | Forge tokens | Raw tokens | Forge token reduction | Forge wall change |
|---|---:|---:|---:|---:|---:|---:|
| GPT-5.6 Luna | 5/6 | 6/6 | 10,339,150 | 10,658,164 | 2.99% lower | 17.51% slower |
| GPT-5.6 Sol | 5/6 | 6/6 | 7,755,014 | 6,599,897 | 17.50% higher | 51.11% slower |
| GPT-5.6 Terra | 6/6 | 6/6 | 9,028,172 | 5,323,356 | 69.60% higher | 113.90% slower |

Forge and raw Codex both resolved all 15 non-Django model-task pairs. On
`django__django-14376`, raw Codex resolved all three models; Forge resolved Terra but not Luna or
Sol.

### Django failure analysis

Both unresolved Forge patches changed the mysqlclient connection kwargs in
`django/db/backends/mysql/base.py`, but omitted the related dbshell compatibility change in
`django/db/backends/mysql/client.py`. The official evaluator applied both patches cleanly, preserved
all PASS_TO_PASS tests, and failed the same three FAIL_TO_PASS dbshell tests:

- `test_options_non_deprecated_keys_preferred`
- `test_options_override_settings_proper_values`
- `test_parameters`

Both raw Codex patches updated both code paths and resolved the task. Forge/Terra also found the
dbshell path and resolved it. The Forge system prompt already requires repository search and
whole-problem verification, so these two outcomes do not isolate a safe general product change;
changing global prompting based on one observed task would be benchmark overfitting.

### Checkpoint-patch defect and post-hoc diagnostic

Thirteen of the 18 predeclared Forge prediction patches contained internal
`.forge/checkpoints/**` snapshot files. SWE-bench applied them without evaluator errors, so they did
not explain the two failed tests, but they made the submitted patches noisy and could expose
irrelevant internal state.

Commit `aa337929` fixed the benchmark capture path to exclude only Forge checkpoint snapshots while
preserving legitimate project files under `.forge/`. SWE workspaces also hide checkpoint/log
artifacts through `.git/info/exclude`; two regression tests cover exclusion and idempotence.

After observing the official result, Luna and Sol on Django were rerun through both arms as an
explicitly **post-hoc diagnostic**:

```text
/home/floris/.local/share/forge/harness-bench-20260726/swe-postfix-checkpoint-hygiene-django-luna-sol-v1/
```

| Post-hoc metric | Forge | Native Codex |
|---|---:|---:|
| Official resolves | 0/2 | 2/2 |
| Total tokens | 908,653 | 1,238,711 |
| Total wall time | 358.925s | 315.693s |
| Checkpoint-contaminated patches | 0/2 | 0/2 |

The two Forge patches shrank from 18,290/23,491 bytes before the fix to 2,075/2,046 bytes after it,
but still omitted the dbshell client change and remained unresolved. This confirms the hygiene fix
without claiming a correctness improvement. The post-hoc outcomes do not replace or enlarge the
predeclared 18-pair sample.

## Quota and stopping rule

- Authorized increase: at most 30 weekly percentage points.
- Fresh provider baseline before the controlled study: 0%.
- Corrected controlled baseline finished at 1%.
- Targeted post-fix run finished at 2%.
- Full post-fix controlled rerun finished at 4%.
- Official 36-arm SWE-bench generation finished at 7%.
- Post-hoc four-arm Django diagnostic also finished at 7%.
- Each runner checks fresh response quota after every trial.
- Missing telemetry fails closed before another trial.
- Helm is queried as requested, but its Codex snapshot is marked stale and is not used to override
  newer response telemetry.

## Verification

After the improvement waves and rebase onto current `origin/main`:

- focused Store/core/CLI regressions passed;
- affected packages: 1,070 tests passed, 14 ignored;
- Clippy passed for all affected packages, targets, and features with warnings denied;
- Ruff and Python byte-compilation passed for benchmark scripts;
- fresh debug Forge 2.10.2 built successfully; and
- rescoring reproduced the corrected controlled totals exactly;
- the official gold evaluator check resolved 6/6 with zero errors; and
- the checkpoint-capture regression tests passed.

Final workspace verification:

- `cargo fmt --all -- --check` passed;
- `cargo clippy --locked --all-targets --all-features -- -D warnings` passed;
- `cargo test --locked --all --all-features` passed: 2,288 tests, 26 ignored, 47 suites;
- `cargo build --release --locked --bin forge` passed: 537 crates compiled;
- Ruff passed over `scripts/harness-bench`;
- Python byte-compilation passed over `scripts/harness-bench`; and
- benchmark Python unit discovery passed: 2/2 tests.

## Limitations

- Controlled tasks have deterministic contracts but are only four fixtures.
- One repeat per pair does not estimate per-task model variance; pairwise consistency and aggregate
  statistics help but do not replace repeated runs.
- Native Codex overhead depends on the installed rules/skills/plugin surface.
- Subscription percentage is coarse and based on provider telemetry, not a public token-weighting
  formula.
- The official SWE-bench subset is intentionally small; report its exact `n`, confidence limits,
  and per-instance outcomes rather than generalizing to the full 500-instance Verified set.
- The checkpoint rerun is post-hoc and has only two pairs. It validates artifact hygiene and
  repeatability of the Django outcome, not a population-level effect.
- Token efficiency is not intelligence. A quality-superiority claim requires a higher official
  resolution outcome, not merely fewer tokens on tasks both harnesses solve.
