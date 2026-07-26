# Forge vs native Codex CLI on GPT-5.6

Status: complete
Study date: 2026-07-26
Current branch: `bench/codex-oauth-gpt56-20260726`

## Executive summary

This study compares Forge's own coding harness with the user's installed native Codex CLI while
holding the authenticated OpenAI account, model, reasoning effort, task, starting repository, and
timeout constant.

The completed study supports a narrower, split conclusion:

1. **Controlled-fixture ceiling:** both harnesses passed all 12 post-fix matched task pairs. Forge
   used 75.93% fewer tokens and 43.75% less wall time on those deterministic fixtures.
2. **Official quality point estimate favors raw Codex:** on the predeclared six-instance
   SWE-bench Verified subset across three models, Forge resolved 16/18 model-task pairs and raw
   Codex resolved 18/18. Both discordances were raw-only Django solves. With only two discordances,
   the exact two-sided paired p-value is 0.5, so the subset is too small to establish a reliable
   quality difference.
3. **Hard-task efficiency is mixed:** Forge used 32.72% fewer tokens in aggregate on the official
   phase, but was lower-token in only 10/18 pairs (`p=0.81452942`) and its bootstrap interval for
   median paired reduction spans zero. Forge took 58.64% more wall time in aggregate.

The evidence therefore does **not** support a claim that Forge has better general coding
intelligence, a higher SWE-bench resolve rate, or a universal speed/token advantage. It does show a
large and repeatable efficiency advantage on the controlled fixtures, plus an aggregate but
statistically inconsistent token reduction on this small official subset.

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
| Total tokens | 2,552,084 | 8,298,661 | Forge 69.25% lower |
| Total wall time | 2,391.720s | 3,784.083s | Forge 36.8% lower |
| Lower-token pairs | 12/12 | 0/12 | Sign test `p=0.00048828` |
| Faster pairs | 11/12 | 1/12 | Sign test `p=0.00634766` |

Median paired token reduction was 70.0%. A seeded 20,000-sample bootstrap gave a 95% interval of
51.44%–74.9% for that median.

### By model

| Model | Forge passes | Raw passes | Forge tokens | Raw tokens | Forge reduction |
|---|---:|---:|---:|---:|---:|
| GPT-5.6 Luna | 4/4 | 4/4 | 918,437 | 2,740,572 | 66.49% |
| GPT-5.6 Sol | 4/4 | 4/4 | 970,691 | 2,578,403 | 62.35% |
| GPT-5.6 Terra | 4/4 | 4/4 | 662,956 | 2,979,686 | 77.75% |

### By scenario

| Scenario | Forge passes | Raw passes | Forge tokens | Raw tokens | Forge reduction |
|---|---:|---:|---:|---:|---:|
| Ordered Go pipeline | 3/3 | 3/3 | 511,615 | 1,990,784 | 74.30% |
| Multifile reservations | 3/3 | 3/3 | 585,210 | 2,721,171 | 78.49% |
| Rust transaction ledger | 3/3 | 3/3 | 540,596 | 1,850,648 | 70.79% |
| TypeScript config recovery | 3/3 | 3/3 | 914,663 | 1,736,058 | 47.31% |

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
| Forge/Luna | yes | 19 | 198,620 | 202.794s |
| Raw/Luna | yes | n/a | 853,318 | 382.761s |
| Forge/Sol | yes | 19 | 237,227 | 237.090s |
| Raw/Sol | yes | n/a | 660,706 | 495.809s |

Matched Forge reductions were 76.7% for Luna and 64.1% for Sol; median 70.4%. Weighted wall-time
reduction across the two pairs was 49.93%.

The targeted pre/post Forge comparison:

| Model | Calls before → after | Tokens before → after | Wall before → after |
|---|---:|---:|---:|
| Luna | 30 → 19 | 392,958 → 198,620 (−49.5%) | 281.740s → 202.794s (−28.0%) |
| Sol | 31 → 19 | 388,378 → 237,227 (−38.9%) | 228.373s → 237.090s (+3.8%) |

The Sol wall time did not improve in this single stochastic rerun despite fewer calls. The report
does not convert the token/call improvement into an unsupported latency claim.

No post-fix runtime artifact contains an autofix warning or a nonexistent-lint attempt. The terminal
stream totals exactly match the complete session ledgers (`post_stream_tokens=0`).

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
| Total tokens | 2,423,206 | 10,067,973 | Forge 75.93% lower |
| Total wall time | 2,262.804s | 4,022.659s | Forge 43.75% lower |
| Lower-token pairs | 12/12 | 0/12 | Sign test `p=0.00048828` |
| Faster pairs | 11/12 | 1/12 | Sign test `p=0.00634766` |

Median paired token reduction was 72.2%; the seeded bootstrap 95% interval was 48.3%–85.74%.

### By model

| Model | Forge passes | Raw passes | Forge tokens | Raw tokens | Forge reduction |
|---|---:|---:|---:|---:|---:|
| GPT-5.6 Luna | 4/4 | 4/4 | 666,374 | 4,705,031 | 85.84% |
| GPT-5.6 Sol | 4/4 | 4/4 | 900,026 | 2,976,855 | 69.77% |
| GPT-5.6 Terra | 4/4 | 4/4 | 856,806 | 2,386,087 | 64.09% |

### By scenario

| Scenario | Forge passes | Raw passes | Forge tokens | Raw tokens | Forge reduction |
|---|---:|---:|---:|---:|---:|
| Ordered Go pipeline | 3/3 | 3/3 | 647,746 | 2,337,966 | 72.29% |
| Multifile reservations | 3/3 | 3/3 | 619,077 | 2,257,040 | 72.57% |
| Rust transaction ledger | 3/3 | 3/3 | 533,086 | 3,106,497 | 82.84% |
| TypeScript config recovery | 3/3 | 3/3 | 623,297 | 2,366,470 | 73.66% |

All 12 Forge terminal stream totals exactly match their complete Store ledgers. No retained
runtime JSONL/stderr contains an autofix warning or missing-lint attempt.

### Forge pre/post comparison

Comparing Forge only avoids attributing raw model variance to the harness fix:

| Forge metric | Baseline | Post-fix | Change |
|---|---:|---:|---:|
| Provider calls | 211 | 195 | −7.6% |
| Tokens | 2,552,084 | 2,423,206 | −5.0% |
| Wall time | 2,391.720s | 2,262.804s | −5.4% |

The causal effect is concentrated where the bug was observed:

| TypeScript Forge metric | Baseline | Post-fix | Change |
|---|---:|---:|---:|
| Provider calls | 75 | 52 | −30.7% |
| Tokens | 914,663 | 623,297 | −31.9% |
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
evaluation-v2/*.json
```

The committed analyzer is
[`scripts/harness-bench/analyze_codex_oauth_swe.py`](../../scripts/harness-bench/analyze_codex_oauth_swe.py).
It requires exactly one official report per arm/model, rejects evaluator errors or incomplete
telemetry, and binds resolution back to each matched trial before computing aggregates.

### Official overall result

| Metric | Forge | Native Codex | Result |
|---|---:|---:|---|
| Official resolves | 16/18 | 18/18 | Raw-only on 2 pairs |
| Evaluator errors | 0 | 0 | Complete |
| Total generation tokens | 15,192,293 | 22,581,417 | Forge 32.72% lower |
| Total tokens / official resolve | 949,518 | 1,254,523 | Forge 24.31% lower |
| Total generation wall time | 8,257.542s | 5,205.356s | Forge 58.64% slower |
| Lower-token pairs | 10/18 | 8/18 | Sign test `p=0.81452942` |
| Faster pairs | 7/18 | 11/18 | Sign test `p=0.48068237` |

Median paired Forge token reduction was 35.97%. Its seeded bootstrap 95% interval was
−29.54%–68.60%, which includes zero. The two quality discordances both favored raw Codex; the exact
two-sided paired McNemar/sign p-value is 0.5.

The "tokens / official resolve" row divides all generation tokens—including unresolved attempts—by
official resolves. It does not erase the cost of Forge's two failures.

### Official result by model

| Model | Forge resolves | Raw resolves | Forge tokens | Raw tokens | Forge token reduction | Forge wall change |
|---|---:|---:|---:|---:|---:|---:|
| GPT-5.6 Luna | 5/6 | 6/6 | 3,869,637 | 10,658,164 | 63.69% lower | 17.51% slower |
| GPT-5.6 Sol | 5/6 | 6/6 | 6,098,243 | 6,599,897 | 7.60% lower | 51.11% slower |
| GPT-5.6 Terra | 6/6 | 6/6 | 5,224,413 | 5,323,356 | 1.86% lower | 113.90% slower |

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
