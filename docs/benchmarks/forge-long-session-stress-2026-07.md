# Forge matched long-session stress benchmark — 2026-07-28

This is the current long-session result. It contains one fully matched comparison between regular
full-mesh Forge auto and native Codex CLI on the same six-prompt workload. Honest review excluded
the first native Claude cell because its synthetic base accidentally tracked one generated
`tests/__pycache__/*.pyc` file. Every source and prompt blob matched, but the base trees were not
byte-for-byte identical. A clean Claude replacement was prepared on the correct tree and failed
before model execution because the local Claude OAuth session had expired. Both Claude artifacts
are retained below; neither is used as a current matched headline cell.

## Verdict

On the fully matched Forge-versus-Codex workload, Forge met the target. The requested three-way
Forge/Codex/Claude conclusion remains unconfirmed until Claude authentication is restored:

| Metric | Forge full mesh auto | Native Codex CLI | Native Claude Code |
|---|---:|---:|---:|
| Quality acceptance | **Passed** | **Passed** | Pending clean run |
| Six turns completed | **6/6** | 6/6 | 0/6 clean replacement |
| Total attempted wall time | **559.469s** | 1,333.317s | 1.417s pre-model failure |
| Raw tokens | **1,698,715** | 15,881,503 | 0 replacement-attempt tokens |
| Cache-adjusted tokens (25% cache cost) | **763,291** | 4,541,599 | 0 replacement-attempt tokens |
| Cache-zero-credit tokens | **451,483** | 761,631 | 0 replacement-attempt tokens |
| Integrity audit | **Passed** | Passed | Not applicable yet |
| Public signatures unchanged | **Yes** | Yes | Not applicable yet |
| Original test support preserved | **Yes** | Yes | Not applicable yet |

Relative to native Codex, Forge was **58.04% faster**, used **89.30% fewer raw tokens**, and used
**83.19% fewer cache-adjusted tokens**, with equal pass/fail quality. Relative to native Claude,
no clean matched claim is currently made.

These are results for one adversarial workload, not population estimates.

## Matched methodology

The accepted Forge and Codex cells used:

- the same reservation-service fixture;
- the same six prompts, in the same order;
- one continuous session with full conversational history;
- the same synthetic one-commit base repository, no remotes, and no future Git objects;
- a 1,500-second per-turn timeout;
- the same visible suite, hidden verifier, API/signature audit, original-test replay, patch capture,
  and external-source trace audit; and
- complete accounting for failed, interrupted, and successful attempts.

Native Codex used `codex-cli 0.145.0`, requested and resolved `gpt-5.6-sol`, and regular `high`
effort. The excluded Claude cell used Claude Code `2.1.220`, requested `opus[1m]`, resolved
`claude-opus-5[1m]`, and passed `--effort high`; Claude's JSON stream did not repeat a resolved
effort identifier, so the requested flag is the retained effort evidence.

Forge used `forge 2.11.0` in genuine regular auto mode:

- no model override;
- no effort override;
- full discovered mesh and normal failover;
- one continuous Forge session; and
- no post-hoc patching of the benchmark workspace.

The valid native Codex cell was not rerun. Only invalid Forge cells were rerun after a material
code change. The clean Claude replacement uses the same source tree hash as Codex and Forge,
`7d17bf17bcd5f9467a305edf6a0a5a50dfa1582b`, but stopped before the model ran.

## Final Forge route and per-turn accounting

| Turn | Effective route(s) | Wall time | Raw tokens | Cache-adjusted |
|---:|---|---:|---:|---:|
| 1 | GPT-5.6 Terra | 104.236s | 177,025 | 90,625 |
| 2 | GPT-5.6 Sol, with Terra in the turn's route history | 62.903s | 177,553 | 96,529 |
| 3 | GPT-5.6 Luna | 115.831s | 266,052 | 181,188 |
| 4 | GPT-5.6 Sol | 124.093s | 332,841 | 148,137 |
| 5 | GPT-5.6 Sol | 98.731s | 471,915 | 160,875 |
| 6 | GPT-5.6 Sol | 36.828s | 273,329 | 85,937 |
| **Model turns** |  | **542.622s** | **1,698,715** | **763,291** |
| **Whole harness arm** | setup, dispatch, and turn overhead included | **559.469s** |  |  |

Turn 2 contains both Terra and Sol usage rows because normal mesh failover occurred inside that
turn. This is part of the tested full-mesh behavior, not a pinned-model cell.

## Quality and integrity acceptance

| Check | Forge result |
|---|---:|
| Visible suite | 14/14 passed |
| Hidden stock-contention calls | 100; exactly 1 winner |
| Hidden duplicate request-ID calls | 100; one reservation/decrement |
| Hidden concurrent cancellations | 100; inventory restored exactly once |
| Injected storage failure | Rollback verified |
| Original tests replayed | 8/8 passed |
| Original tests weakened or skipped | No |
| Existing public signatures changed | No |
| Persisted tool executions | 74/74 OK |
| External-solution-like tool calls | 0 |
| Repository remotes | 0 |
| Reachable base commits | 1 |
| Patch `git diff --check` | Passed |

The excluded native Claude cell's visible suite passed, but its hidden verifier failed because a
reused request ID with a different unknown SKU raised `NotFound` before checking the existing
request and raising the contractually required `Conflict`. The first Forge attempt made the same
mistake. Native Codex and the final Forge cell passed this case. This remains useful diagnostic
evidence, but it is not promoted to a matched Claude benchmark result.

## Complete validity and supersession ledger

No attempt was deleted or relabelled as successful:

| Cell or attempt | Wall time | Raw / adjusted tokens | Status | Reason |
|---|---:|---:|---|---|
| Native Codex Sol high | 1,333.317s | 15,881,503 / 4,541,599 | **Valid, passed** | Clean trace and full acceptance |
| Native Claude Opus 5 high, original | 1,033.773s | 5,660,416 / 1,659,035.5 | **Invalid/superseded** | Source-equivalent, but base commit tracked one generated `.pyc`; hidden conflict-precedence failure retained as diagnostic evidence |
| Native Claude Opus 5 high, clean replacement | 1.417s | 0 / 0 | **No benchmark cell** | Exact matched base; expired OAuth stopped before model execution |
| Forge attempt 1 | 1,074.668s | 2,852,727 / 1,887,927 | **Invalid/superseded** | Hidden conflict-precedence failure |
| Forge attempt 2 | 758.737s | 1,634,531 / 616,163 | **Invalid/superseded** | Behavior passed, but added a public method |
| Forge attempt 3 | 559.469s | 1,698,715 / 763,291 | **Valid, passed** | Every acceptance and integrity gate passed |

The three matched Forge development/confirmation attempts consumed 2,392.874 seconds,
6,185,973 raw tokens, and 3,267,381 cache-adjusted tokens in total. Those costs are disclosed
separately from the final steady-state performance comparison.

## Honest review of the stress work

### 1. Verified findings

- The accepted Codex and Forge base trees are byte-identical at
  `7d17bf17bcd5f9467a305edf6a0a5a50dfa1582b`; their six prompt hashes also match.
- The accepted Forge patch passes the visible suite, every exact hidden invariant, original-test
  replay, complete public-signature comparison, trace audit, and `git diff --check`.
- The headline arithmetic was independently recomputed from retained summaries.
- All failed/interrupted Forge time remains in the attempt ledger; the clean Claude authentication
  failure is retained with its zero-token telemetry.

### 2. Potential hallucinations

None found after correction. Exact CLI versions, models, effort flags, routes, quotas, times, and
tokens are present in retained machine-readable artifacts.

### 3. Unsupported claims

The former three-way headline said the original Claude cell had the same base repository. That was
too strong: all source blobs matched, but one generated `.pyc` was tracked in Claude's base commit.
The Claude figures were removed from the matched headline and the cell was marked superseded.

### 4. Weak assumptions

- Zero events and zero tokens were previously enough to classify a native CLI failure as a free
  parser failure. That could misclassify a provider/transport failure and permit a duplicate call.
  Recovery now requires a short exit-code-2 failure plus explicit local usage/parser evidence.
- Forge finalization previously treated `rollback_verified` alone as the hidden-test gate. It now
  requires the exact 100-request contention, duplicate, cancellation, one-winner, and rollback
  values.
- Quota summaries previously accepted decreasing or out-of-order observations. They now reject
  non-monotonic, out-of-range, pre-baseline, reversed-time, and over-cap evidence.

### 5. Missing evidence

A clean native Claude quality/time/token result is missing because local OAuth expired before model
execution. No current claim compares Forge with native Claude on the exact tree.

### 6. Alternative explanations

The excluded Claude semantic failure is one stochastic sample; the generated bytecode mismatch
does not explain it because every source/test blob was identical, but the mismatch still prevents
calling the arm exact. Likewise, Forge's first miss could reflect both the weaker initial route and
model stochasticity; the minimum reruns establish improvement, not a universal causal effect size.

### 7. Confidence assessment

Confidence is high for the narrow Forge-versus-Codex result and for the final Forge acceptance
state. Confidence is intentionally withheld for Forge-versus-Claude until a clean authenticated
replacement completes.

### 8. Overall reliability score

**Medium overall.** The accepted Forge/Codex comparison is well-supported, but the requested
three-way conclusion is incomplete. Consumers should rely on the Forge/Codex numbers only.

### 9. Recommended corrections

1. Restore native Claude authentication and resume only the prepared exact-tree replacement.
2. Refresh Helm before and after every turn and stop at the existing 32% hard limit.
3. Replace the pending Claude column only after all six turns, integrity checks, exact hidden
   acceptance, patch capture, and final quota refresh pass.

The audit also made patch artifacts source-only by excluding `.forge`, `.claude`, and interpreter
cache noise while retaining all committed and uncommitted source/test changes. Future manifests
record the Git tree hash directly. Completion evidence now rejects `set +e`, negated checks,
backgrounded checks, conditional masking, and pipelines that merely mention rather than enable
`pipefail`. All of these corrections have deterministic regression coverage.

## Diagnose → fix → retest loops

### 1. Complex task-defining turns could conserve onto a materially weaker model

The first matched Forge attempt correctly classified the task as Complex but routed its initial
implementation to Qwen 3.6 Flash. Its measured coding score was 69.2 versus 77.4 for GPT-5.6 Sol,
and the resulting implementation missed conflict precedence. A catalog-only conservation guard
had seen a high-quality but unavailable alternative, fired conservation, then allowed a weaker
healthy model to lead the real usable chain.

Fix:

- the first turn of a Complex coding task is now a quality anchor;
- it selects the strongest measured model among candidates that are actually usable after health,
  context, credit-mode, and OAuth-twin filtering;
- dependent continuation turns retain normal quota-, speed-, and score-aware diversification; and
- the `/mesh` inspector overlays the same effective ordering, so its selected row is rank 1 rather
  than a misleading low-ranked catalog row.

Free regression tests reproduced the unavailable-near-peer case and proved that the initial turn
uses the strongest measured usable model while a continuation still conserves normally. The next
paid cell passed the hidden semantic check.

### 2. Final review narrowed "public API" to a hand-picked subset

The next Forge cell passed every behavioral test, but Sol added
`InMemoryStore.rollback_sequence()`. Its final review checked only the three
`ReservationService` methods and package exports, then incorrectly claimed that the public surface
was unchanged. The exact public-signature audit failed closed.

Fix:

- the turn contract now detects explicit public-API/signature-preservation requirements;
- provider-visible guidance states that additions are changes too; and
- the model must compare the complete public surface against the base rather than auditing only
  edited or named methods.

The exact final-prompt regression passed locally. In the minimum paid retest, Forge avoided the
new helper, the full public-signature comparison passed, and all behavioral checks still passed.

### 3. Earlier endurance findings retained in this branch

The broader stress work also fixed and regression-tested:

- bounded long-history context fitting without eagerly tokenizing discarded history;
- queued reprompts steering the next loop/goal iteration while retaining FIFO order;
- stale completion markers not completing a replacement turn;
- interrupted same-session recovery and committed/uncommitted patch capture; and
- durable six-prompt PTY timing, session, integrity, and acceptance artifacts.

## Quota ledger

The original native cells and first Forge attempt ran before the Codex weekly window reset:

- Codex: recorded 36% baseline to 39% final, **+3 points**;
- Claude: recorded 27% baseline to 28% final, **+1 point**.

Helm then observed a real Codex weekly reset to 0% with a new `2026-08-04` reset time. The hard
limit for the new window was therefore recorded as 5%. Across both post-reset Forge confirmation
arms:

- Codex moved from 0% to 1%, **+1 point** against the 5-point cap;
- Claude stayed at 28%, **+0 points** against the existing 32% hard stop.

Helm was refreshed before and after each paid arm. Forge's direct Codex OAuth quota event at
`2026-07-28T03:48:04Z` independently confirmed the post-arm 1% observation after the final model
turn. Percentage telemetry is rounded, so token totals remain the more precise work measure.

The clean Claude replacement was attempted at a retained 28% weekly utilization. It failed before
model execution with zero reported tokens, Helm was refreshed afterward, and weekly utilization
remained 28% against the 32% hard stop. The runner preserved the failure and will not silently retry
it.

## Workload shape

The scenario was derived from aggregate-only local history statistics, not prompt contents:

| Aggregate | Codex | Claude Code |
|---|---:|---:|
| Sessions scanned | 870 | 1,735 |
| Forge-project sessions | 451 | 1,354 |
| User turns | 1,864 | 4,601 |
| Tool calls | 31,993 | 66,878 |
| Compactions | 127 | 120 |
| Largest session by user turns | 94 | 654 |
| Largest session by tool calls | 5,510 | 6,995 |
| Largest session by compactions | 22 | 34 |

The synthetic workload escalates through initial repair, high-contention/idempotency tests,
rollback and cancellation, long-running-state review, skeptical diff review, and final
verification.

## Verification

After the honest-review corrections and report edits:

- `cargo test --workspace --all-targets` passed;
- `cargo clippy --workspace --all-targets -- -D warnings` passed;
- `cargo fmt --all -- --check` passed;
- all 197 mesh tests passed;
- all 35 manual-harness Python tests passed;
- the 650-turn / 7,800-message routing endurance test passed;
- three long-session core endurance tests passed; and
- TUI replay, queued-reprompt, provider-bridge, integrity, and patch-capture regressions passed.

`bash -n`, Python compilation, and `git diff --check` also passed. The first full workspace run
exposed one overly strict `pipefail` regression; that finding was fixed, its focused module passed,
and the complete workspace suite was rerun successfully rather than hiding the failed gate.

## Honest limitations

- One six-prompt task cannot prove superiority on every repository or stochastic run.
- The original Claude sample is excluded from the matched headline because of its generated
  bytecode base-tree mismatch; the clean replacement is pending authentication.
- The valid Forge cell routed only within GPT-5.6 because those were the strongest healthy measured
  candidates under the live catalog and quota state; a different account state can produce a
  different auto route.
- Cache-adjusted tokens charge cached input at 25%; cache-zero-credit tokens are also shown because
  subscription accounting differs by provider.
- Final-run wall time is the steady-state comparison. The full development/retest cost is disclosed
  separately and must not be mistaken for product latency.

## Reproduction

Free/local regression coverage:

```bash
python3 -m unittest -v \
  scripts/manual-e2e/test_native_long_session.py \
  scripts/manual-e2e/test_profile_agent_history.py \
  scripts/manual-e2e/test_pty_chat_harness.py \
  scripts/manual-e2e/test_summarize_forge_long_session.py \
  scripts/manual-e2e/test_verify_session_tools.py
cargo test -p forge-agent-mesh
cargo test -p forge-agent-core --test long_session_endurance
cargo test -p forge-agent-tui --test long_session_replay
cargo test -p forge-agent queued_reprompts
```

The live scenario consumes provider quota:

```bash
scripts/manual-e2e/run.sh long-session-reservations
```

Retained run artifacts live outside the repository under
`${XDG_DATA_HOME:-$HOME/.local/share}/forge/manual-e2e-runs/`.
