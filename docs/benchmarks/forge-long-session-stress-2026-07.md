# Forge matched long-session stress benchmark — 2026-07-28

This is the current long-session result. It contains a fully matched comparison between regular
full-mesh Forge auto, native Codex CLI, and native Claude Code on the same six-prompt workload.
Honest review excluded the first native Claude cell because its synthetic base accidentally tracked
one generated `tests/__pycache__/*.pyc` file. The clean Claude replacement used the exact accepted
tree, completed all six Opus 5 turns after authentication was restored, and is the only Claude cell
used in the headline.

A follow-up skeptical review strengthened the hidden acceptance with 100 deterministically
alternating reserve/cancel operations and injected cancellation-write rollback. Free replays passed
on the retained valid Forge and Codex workspaces; no paid rerun was needed. The same review fixed a remote
interrupt state leak, repeated Claude stream-snapshot output, and weak native quota-ledger
validation. Those fixes and replays are recorded below.

## Verdict

On this fully matched workload, Forge met the target against both native harnesses:

| Metric | Forge full mesh auto | Native Codex CLI | Native Claude Code |
|---|---:|---:|---:|
| Quality acceptance | **Passed** | **Passed** | Failed hidden conflict precedence |
| Six turns completed | **6/6** | 6/6 | 6/6 |
| Total attempted wall time | **559.469s** | 1,333.317s | 1,019.400s |
| Raw tokens | **1,698,715** | 5,129,177 | 4,641,031 |
| Cache-adjusted tokens (25% cache cost) | **763,291** | 1,430,681 | 1,362,289.75 |
| Cache-zero-credit tokens | 451,483 | **197,849** | 269,376 |
| Integrity audit | **Passed** | Passed | Passed |
| Public signatures unchanged | **Yes** | Yes | Yes |
| Original test support preserved | **Yes** | Yes | Yes |

Relative to native Codex, Forge was **58.04% faster**, used **66.88% fewer raw tokens**, and used
**46.65% fewer cache-adjusted tokens**, with equal pass/fail quality. Native Codex used 56.18% fewer
uncached-input-plus-output tokens under the cache-zero sensitivity, so Forge does not win every
token-accounting model.

Relative to native Claude, Forge passed the exact hidden contract that Claude missed, was
**45.12% faster**, used **63.40% fewer raw tokens**, and used **43.97% fewer cache-adjusted
tokens**. Native Claude used 40.34% fewer cache-zero-credit tokens than Forge, so the same
cache-accounting caveat applies.

These are results for one adversarial workload, not population estimates.

## Matched methodology

The accepted Forge and Codex cells and the valid failed-quality Claude cell used:

- the same reservation-service fixture;
- the same six prompts, in the same order;
- one continuous session with full conversational history;
- the same synthetic one-commit base repository, no remotes, and no future Git objects;
- a 1,500-second per-turn timeout;
- the same visible suite, hidden verifier, API/signature audit, original-test replay, patch capture,
  and external-source trace audit; and
- complete accounting for failed, interrupted, and successful attempts.

Native Codex used `codex-cli 0.145.0`, requested and resolved `gpt-5.6-sol`, and regular `high`
effort. Native Claude used Claude Code `2.1.220`, requested `opus[1m]`, resolved only
`claude-opus-5[1m]` on every turn, and passed `--effort high`; Claude's JSON stream does not repeat
a resolved effort identifier, so the immutable argv is the retained effort evidence.

Forge used `forge 2.11.0` in genuine regular auto mode:

- no model override;
- no effort override;
- full discovered mesh and normal failover;
- one continuous Forge session; and
- no post-hoc patching of the benchmark workspace.

The valid native Codex cell was not rerun. Only invalid Forge cells were rerun after a material
code change. The clean Claude replacement used the same source tree hash as Codex and Forge,
`7d17bf17bcd5f9467a305edf6a0a5a50dfa1582b`, and all six prompt hashes matched. Its two
pre-provider recovery attempts are included in attempted wall time but reported zero tokens.

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

## Corrected native Codex per-turn accounting

Codex CLI 0.145.0 reports `turn.completed.usage` as a thread-cumulative snapshot. The original
summary incorrectly summed all six snapshots. The corrected runner retains each cumulative value
but subtracts the preceding snapshot before aggregating:

| Turn | Wall time | Raw token delta | Cache-adjusted delta | Cache-zero delta |
|---:|---:|---:|---:|---:|
| 1 | 239.607s | 569,973 | 183,477 | 54,645 |
| 2 | 151.181s | 437,965 | 119,053 | 12,749 |
| 3 | 301.033s | 883,144 | 249,544 | 38,344 |
| 4 | 286.647s | 1,129,619 | 316,307 | 45,203 |
| 5 | 242.608s | 1,241,931 | 336,075 | 34,123 |
| 6 | 112.225s | 866,545 | 226,225 | 12,785 |
| **Model turns** | **1,333.301s** | **5,129,177** | **1,430,681** | **197,849** |
| **Whole attempted arm** | **1,333.317s** |  |  |  |

The 0.016-second difference is the retained local CLI-argument preflight failure. Each cumulative
snapshot exactly matches the session rollout's `token_count.info.total_token_usage` at that turn
boundary. The [machine-readable recalculation](artifacts/long-session-codex-token-recalculation-2026-07.json)
pins the source hashes, original superseded totals, deltas, and corrected comparison.

## Native Claude per-turn accounting

| Turn | Wall time | Raw tokens | Cache-adjusted | Cache-zero |
|---:|---:|---:|---:|---:|
| 1 | 91.388s | 431,848 | 149,551 | 55,452 |
| 2 | 288.910s | 826,548 | 251,238 | 59,468 |
| 3 | 157.281s | 536,784 | 190,566.75 | 75,161 |
| 4 | 222.095s | 1,176,163 | 323,834.5 | 39,725 |
| 5 | 211.233s | 1,413,346 | 378,242.5 | 33,208 |
| 6 | 46.811s | 256,342 | 68,857 | 6,362 |
| **Model turns** | **1,017.718s** | **4,641,031** | **1,362,289.75** | **269,376** |
| **Whole attempted arm** | **1,019.400s** |  |  |  |

The additional 1.682 seconds are the retained 1.417-second expired-authentication attempt and
0.265-second local session-ID collision. Both stopped before model execution and reported no
tokens. Authentication recovery rotated the never-started session ID; every actual model turn then
used one continuous replacement session. The
[machine-readable Claude ledger](artifacts/long-session-native-claude-clean-2026-07.json) pins the
exact setup, per-turn accounting, failed attempts, acceptance result, and quota delta.

## Quality and integrity acceptance

| Check | Forge result |
|---|---:|
| Visible suite | 14/14 passed |
| Hidden stock-contention calls | 100; exactly 1 winner |
| Hidden duplicate request-ID calls | 100; one reservation/decrement |
| Hidden concurrent cancellations | 100; inventory restored exactly once |
| Hidden alternating reserve/cancel operations | 100; 50 old inactive, 50 replacements active; inventory exact |
| Injected storage failure | Rollback verified |
| Injected cancellation storage failure | Reservation and inventory rollback verified |
| Original tests replayed | 8/8 passed |
| Original tests weakened or skipped | No |
| Existing public signatures changed | No |
| Persisted tool executions | 74/74 OK |
| External-solution-like tool calls | 0 |
| Repository remotes | 0 |
| Reachable base commits | 1 |
| Patch `git diff --check` | Passed |

The clean native Claude cell passed 23/23 visible tests and replayed all 8 original tests, but its
hidden verifier failed because a reused request ID with a different unknown SKU raised `NotFound`
before checking the existing request and raising the contractually required `Conflict`. The first
Forge attempt and superseded Claude cell made the same mistake. Native Codex and final Forge passed
this exact case. The clean Claude cell remains a valid matched measurement with a failed quality
outcome; it is not relabelled invalid merely because it lost.

## Complete validity and supersession ledger

No attempt was deleted or relabelled as successful:

| Cell or attempt | Wall time | Raw / adjusted tokens | Status | Reason |
|---|---:|---:|---|---|
| Native Codex Sol high | 1,333.317s | 5,129,177 / 1,430,681 | **Valid, passed** | Clean trace, full acceptance, cumulative usage corrected from retained events |
| Native Claude Opus 5 high, original | 1,033.773s | 5,660,416 / 1,659,035.5 | **Invalid/superseded** | Source-equivalent, but base commit tracked one generated `.pyc`; hidden conflict-precedence failure retained as diagnostic evidence |
| Native Claude Opus 5 high, clean replacement | 1,019.400s | 4,641,031 / 1,362,289.75 | **Valid, failed quality** | Exact tree and setup; 6/6 turns, clean integrity, hidden conflict-precedence failure |
| Forge attempt 1 | 1,074.668s | 2,852,727 / 1,887,927 | **Invalid/superseded** | Hidden conflict-precedence failure |
| Forge attempt 2 | 758.737s | 1,634,531 / 616,163 | **Invalid/superseded** | Behavior passed, but added a public method |
| Forge attempt 3 | 559.469s | 1,698,715 / 763,291 | **Valid, passed** | Every acceptance and integrity gate passed |

The three matched Forge development/confirmation attempts consumed 2,392.874 seconds,
6,185,973 raw tokens, and 3,267,381 cache-adjusted tokens in total. Those costs are disclosed
separately from the final steady-state performance comparison.

## Honest review of the stress work

### 1. Verified findings

- The accepted Codex and Forge and clean Claude base trees are byte-identical at
  `7d17bf17bcd5f9467a305edf6a0a5a50dfa1582b`; all six prompt hashes also match.
- The accepted Forge patch passes the visible suite, every exact hidden invariant, original-test
  replay, complete public-signature comparison, trace audit, and `git diff --check`.
- The strengthened hidden verifier was replayed against both retained accepted workspaces: Forge
  and native Codex each passed 100 deterministically alternating reserve/cancel operations and
  cancellation-write rollback without another provider call.
- The headline arithmetic was independently recomputed from retained summaries.
- The corrected Codex token total telescopes to its final cumulative snapshot; every snapshot also
  equals the rollout's retained cumulative token count at the same turn boundary.
- All failed/interrupted Forge time remains in the attempt ledger. The clean Claude cell retains
  both its zero-token authentication failure and zero-event session-ID collision alongside every
  successful turn.

### 2. Potential hallucinations

None found after correction. Exact CLI versions, models, effort flags, routes, quotas, times, and
corrected token deltas are present in retained machine-readable artifacts.

### 3. Unsupported claims

The former three-way headline said the original Claude cell had the same base repository. That was
too strong: all source blobs matched, but one generated `.pyc` was tracked in Claude's base commit.
That cell remains superseded; only the exact-tree replacement is now promoted. The former Codex
token headline also summed cumulative snapshots as if they were per-turn usage. Those totals and
the resulting 89.30%/83.19% efficiency claims are superseded by the corrected 66.88% raw and
46.65% cache-adjusted reductions.

### 4. Weak assumptions

- Zero events and zero tokens were previously enough to classify a native CLI failure as a free
  parser failure. That could misclassify a provider/transport failure and permit a duplicate call.
  Recovery now requires a short exit-code-2 failure plus explicit local usage/parser evidence.
- Forge finalization previously treated `rollback_verified` alone as the hidden-test gate. It now
  requires the exact 100-request contention, duplicate, cancellation, one-winner, and rollback
  values.
- Quota summaries previously accepted decreasing or out-of-order observations. They now reject
  non-monotonic, out-of-range, pre-baseline, reversed-time, and over-cap evidence.
- The native runner still checked only the newest quota observation, so a later decreasing or
  out-of-order sample could understate final native usage. It now validates the entire retained
  ledger before recording a sample, starting a turn, or finalizing.
- The native runner assumed Codex `turn.completed.usage` was scoped to one invocation. Retained
  rollout events prove it is thread-cumulative in Codex CLI 0.145.0. It now stores the authoritative
  cumulative snapshots and aggregates only their monotonic deltas.
- Native finalization recorded resolved model/effort identifiers but did not make them acceptance
  gates, and it did not recheck the prepared CLI version before resumed turns. A transparent model
  substitution or mid-run CLI upgrade could therefore retain a green, wrongly attributed result.
  New runs require one exact expected resolved model, regular `high`, and the prepared CLI version
  on every turn. Codex must also report resolved `high`; Claude's immutable `--effort high` argv
  remains the evidence because Claude Code 2.1.220 does not repeat effort in stream JSON.
- Retry directory naming counted recovered parser failures but not recovered authentication
  failures. Preserving the original zero-token `turns/01` artifact therefore caused the restored
  run to stop locally when it tried to create `turns/01` again. Naming now counts both ledgers and
  uses `turns/01-attempt-02`; the collision happened before any second provider process started.
- Claude reserves a caller-supplied session ID locally even when authentication fails before model
  execution. Recovery now rotates that never-started ID, retains the mapping, and treats only the
  exact short zero-event `Session ID … is already in use` error as a recoverable local failure.
- Claude's authoritative subscription samples may be emitted inside the final provider request
  rather than after CLI exit. Quota gating now separately requires a sample from the completed arm
  and a Helm retrieval/ledger write after process exit.
- `git add -N .` can fail on an ignored empty `.claude` directory even when its contents are
  excluded by pathspec. Patch capture now inventories only non-ignored untracked files and applies
  intent-to-add to those exact paths.

### 5. Missing evidence

No required cell is missing. Statistical replication across more repositories and random seeds is
still absent; this is one exact workload and one clean run per final harness.

### 6. Alternative explanations

The clean Claude semantic failure is one stochastic sample. Its independent superseded run made the
same mistake, which strengthens the diagnostic signal but still does not establish a population
failure rate. Likewise, Forge's first miss could reflect both the weaker initial route and model
stochasticity; the minimum reruns establish improvement on this cell, not a universal causal effect
size.

### 7. Confidence assessment

Confidence is high for the narrow three-way result and final Forge acceptance state because the
trees, prompts, traces, identities, evaluations, and quotas are matched and audited. Confidence in
generalization beyond this workload remains limited.

### 8. Overall reliability score

**High internal validity; limited external validity.** The requested three-way cell is complete and
well-supported, but one workload cannot prove universal superiority.

### 9. Recommended corrections

1. Repeat the same protocol on additional long-session workloads only when quota permits.
2. Keep raw, 25%-adjusted, and cache-zero sensitivity separate rather than collapsing token
   efficiency to one accounting model.
3. Preserve failed-quality cells as valid measurements when setup and integrity are clean.

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

### 4. Remote interrupts left workflow state active

The local interrupt path explicitly closed an active workflow because its aborted task could no
longer emit `WorkflowFinished`. The TUI's remote interrupt path did not. A remote correction could
therefore start its queued replacement turn while the old workflow status band remained active,
causing later workflow events to attach to stale state.

Fix:

- local and remote interrupts now share one presenter-finalization helper;
- the helper closes the workflow before flushing the interrupted assistant response; and
- a deterministic regression starts a workflow, interrupts it through the shared path, and proves
  that both `active` and the full-screen overlay are closed with an interrupted result.

This path is independent of provider output, so the focused free regression is the minimum
affected retest; repeating a paid six-turn cell would not exercise it.

### 5. Claude block snapshots could repeat an already-emitted suffix

Claude Code 2.1.220 emits block-level `assistant` snapshots before the enclosing streamed message
ends. The first deduplication fix matched a snapshot to its original stream index, but cleared the
state after each snapshot. A repeated snapshot could therefore emit the unstreamed suffix twice.

Fix:

- text, reasoning, and tool-ID stream state is now bounded by explicit `message_start` /
  `message_stop` events rather than each block snapshot;
- consolidated snapshots advance the retained content for their actual stream index; and
- regressions cover partial suffixes, block-index remapping, repeated text and tool snapshots, and
  identical text in a later genuine message.

All 29 focused Claude bridge tests passed. The accepted Forge cell routed only through GPT-5.6, so
no unchanged paid result was rerun for this bridge-only correction.

### 6. Native quota finalization trusted only the newest sample

The Forge summary rejected decreasing quota observations, but the native runner validated only its
last row. A malformed sequence could therefore report a smaller final delta or hide out-of-order
external evidence.

Fix:

- every retained observation must now be finite, in range, at or above the fixed baseline, and
  monotonic in recorded time, external observation time, completed-turn index, and utilization;
- a weekly reset inside a cell fails closed and requires a newly prepared baseline rather than
  silently granting a second allowance; and
- an invalid observation is rejected before it can modify the durable run state.

Deterministic tests prove decreasing and reordered ledgers are denied and that rejection leaves the
saved ledger unchanged. This changes only accounting gates, not model behavior, so no paid rerun
was justified.

### 7. Hidden acceptance did not enforce all of the rollback prompt

Turn 3 requested concurrent reserve/cancel interleavings and cancellation rollback, but the hidden
verifier checked only reserve contention, duplicate IDs, repeated cancellation after reservation,
and reservation-write rollback. A cell could pass while ignoring part of the prompt.

Fix:

- the hidden verifier holds the shared lock while queueing 50 cancellations and 50 replacement
  reservations in exact alternating order, records their actual persistence order, and requires the
  old reservations inactive, replacements active, and inventory exact; and
- it injects a cancellation storage failure and requires both the original active reservation and
  pre-cancellation inventory to survive.

Free replay results:

| Retained cell | Strengthened hidden result |
|---|---|
| Forge mesh auto, accepted attempt 3 | **Passed** |
| Native Codex Sol high | **Passed** |
| Native Claude Opus 5, superseded | Failed earlier at the already-recorded conflict-precedence case |

Because the two accepted patches passed the stronger evaluator unchanged, their quality, time, and
token cells remain valid without spending quota on identical model calls. The
[machine-readable replay ledger](artifacts/long-session-hidden-replay-2026-07.json) pins the
verifier hash, source patch hashes, base trees, and exact results.

### 8. Native Codex cumulative usage was counted once per turn

The retained Codex `turn.completed.usage` values rose monotonically to 5,129,177 raw tokens. The
runner had treated each value as invocation-local and summed them to 15,881,503. Inspection of the
authoritative rollout proved that every value exactly matched
`token_count.info.total_token_usage`, which is cumulative across the continuous thread.

Fix:

- the runner now preserves each cumulative snapshot separately;
- turn accounting subtracts the preceding cumulative input, cached-input, and output fields;
- decreasing or incomplete cumulative telemetry fails closed instead of producing a plausible
  total; and
- a deterministic regression proves that six snapshots telescope to the final cumulative value
  rather than summing repeated history.

No provider rerun was needed. Recalculation from the hashed retained events corrected native Codex
to 5,129,177 raw, 1,430,681 cache-adjusted, and 197,849 cache-zero-credit tokens. Forge still meets
the raw and cache-adjusted efficiency target, but the report now discloses that native Codex wins the
cache-zero sensitivity.

### 9. A clearly logged-out Claude CLI was not denied before process start

The clean replacement preserved its zero-token authentication failure correctly, but the runner
relied on the provider process to discover that the local CLI was logged out. That spent an
avoidable process attempt and created a recovery step even though `claude auth status --json` can
fail closed before model transport starts.

Fix:

- every native Claude turn now checks local CLI authentication after the fresh quota gate and before
  creating or launching a provider process;
- a logged-out status, malformed response, timeout, or missing CLI denies the call; and
- a deterministic regression proves `run_capture` is never reached when authentication is absent.

After the user restored authentication, the exact-tree replacement resumed without duplicating the
zero-token attempt and completed all six model turns.

### 10. Cache-free sensitivity favors native Codex

After cumulative-token correction, native Codex used 197,849 uncached-input-plus-output tokens
versus Forge's 451,483. The per-call ledger explains the direction: native stayed on Sol and kept a
92–99% input-cache ratio, while regular mesh used Terra, Sol, and Luna across its first three turns.
Those three models' first-use turns account for 270,435 of Forge's 423,299 uncached input tokens;
after the route stayed on Sol, its per-turn cache ratio rose to 89–92%.

Correction:

- the headline no longer implies Forge wins every cache accounting model;
- raw and 25%-adjusted totals remain the primary comparable efficiency measures, and the
  cache-zero sensitivity is displayed alongside them; and
- no speculative stickiness patch was made. Forcing one model across dependent turns would change
  genuine regular auto routing and could trade away measured speed, quota balance, or quality. The
  retained evidence cannot prove that such a policy is better, and a cache-free sensitivity is not
  a provider's actual processing or billing model.

Forge still exceeds the stated overall efficiency target on raw and cache-adjusted tokens. A future
controlled routing experiment may test quality-bounded model affinity, but it is not smuggled into
this already-completed cell.

### 11. Native execution identity was recorded but not acceptance-gated

The native runner retained requested and resolved models, yet its final `all_acceptance_passed`
boolean did not depend on them. It also recorded CLI version only during preparation. That was
sufficient for manual inspection of these retained cells but not a defensible fail-closed harness:
a CLI alias/provider substitution or mid-session CLI update could change the execution identity and
still receive a green result attributed to the original setup.

Fix:

- every prepared run now stores one exact `expected_resolved_model`;
- only regular `high` is accepted, and every turn must use the prepared CLI version;
- Codex turns must report that model and resolved `high`;
- Claude turns must report exactly that one model through `modelUsage`; and
- recovery of a legacy zero-token Claude authentication failure requires the exact expectation
  before the paid run can resume.

Deterministic tests cover matching, wrong-model, wrong-effort, version drift, missing-evidence, and
legacy-recovery paths. The retained Codex trace already proves `codex-cli 0.145.0`,
`gpt-5.6-sol`, and `high` on all six turns, so this accounting-gate correction does not justify
another provider call. The clean Claude trace then proved Claude Code `2.1.220`,
`claude-opus-5[1m]`, and immutable `--effort high` on all six turns.

### 12. Recovered authentication evidence collided with the retry directory

After login was restored, `recover-auth` correctly retained the original 1.417-second zero-token
failure. The next `turn` command nevertheless counted only `preflight_failures` when choosing its
artifact directory, attempted to recreate the retained `turns/01`, and stopped at local
`mkdir(exist_ok=False)` before `run_capture`.

Fix:

- retry naming now counts unique retained directories from both `preflight_failures` and
  `authentication_failures`;
- the preserved authentication failure therefore advances the retry to `01-attempt-02`; and
- deterministic coverage proves the recovered turn contributes exactly one retained attempt
  without affecting another turn.

The run state remained at turn 0 with no new paid failure or provider process, so resuming the same
prepared cell after this correction is not a duplicate call.

### 13. Failed authentication reserved the local Claude session ID

The next local attempt used the correctly named `01-attempt-02` directory but Claude Code rejected
the original UUID with `Session ID … is already in use`. The expired-authentication process had
reserved that UUID locally despite reporting zero model tokens.

Fix:

- zero-token authentication recovery now rotates the never-started UUID and records the old/new
  mapping;
- the already-recovered cell recognizes only the exact short, zero-event local collision as
  recoverable and rotates once; and
- all actual model turns must then report the replacement UUID.

The collision stopped in 0.265 seconds before a provider event. It is preserved in attempted time.
The six successful turns all used the one replacement session.

### 14. Claude quota samples and ignored directories violated harness assumptions

Two finalization assumptions failed locally:

- Claude can emit its authoritative weekly sample inside the last provider request rather than
  after CLI exit. The quota gate now proves the sample belongs to the completed arm and separately
  proves the Helm retrieval was recorded after process exit. The final no-model `/usage` refresh
  produced an explicitly post-turn Helm sample.
- `git add -N .` failed on Claude Code's ignored empty `.claude/.cc-writes` directory. Patch capture
  now asks Git for non-ignored untracked paths and applies intent-to-add only to those exact files;
  tracked edits and all non-ignored new source/tests remain in the binary patch.

Deterministic tests reproduce both cases. Neither correction changes model output or justifies a
provider rerun.

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
The user later authorized a fresh +10-point Codex allowance after reset, but the retained native
Codex cell was already valid and no Codex-backed rerun was needed or launched.

The clean Claude replacement's first model turn started at a retained 28% weekly utilization and
the final post-turn Helm observation was 29%. Against the fixed 27% baseline, the complete retained
ledger ended at **+2 points**, below the 32% hard stop; the six-turn replacement itself moved the
rounded reading by **+1 point**. Helm was refreshed after every turn. The initial 1.417-second
authentication failure and 0.265-second session collision reported zero tokens and remain included
in attempted wall time.

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

- `cargo test --workspace --all-targets` passed: 2,336 tests, with 28 intentional ignores;
- `cargo clippy --workspace --all-targets -- -D warnings` passed;
- `cargo fmt --all -- --check` passed;
- `cargo build --release --locked --bin forge` passed;
- 196 mesh tests passed and 1 intentionally ignored test remained ignored;
- all 39 manual-harness Python tests passed;
- strengthened hidden verification passed for Forge and Codex and failed honestly for clean native
  Claude at conflict precedence;
- the 650-turn / 7,800-message routing endurance test passed;
- three long-session core endurance tests passed; and
- TUI replay, queued-reprompt, provider-bridge, integrity, and patch-capture regressions passed.

`bash -n`, Python compilation, and `git diff --check` also passed. The first full workspace run
exposed one overly strict `pipefail` regression; that finding was fixed, its focused module passed,
and the complete workspace suite was rerun successfully rather than hiding the failed gate.

## Honest limitations

- One six-prompt task cannot prove superiority on every repository or stochastic run.
- The original Claude sample remains excluded because of its generated-bytecode base-tree
  mismatch; the clean replacement is the headline Claude cell and failed one hidden invariant.
- The valid Forge cell routed only within GPT-5.6 because those were the strongest healthy measured
  candidates under the live catalog and quota state; a different account state can produce a
  different auto route.
- Cache-adjusted tokens charge cached input at 25%; cache-zero-credit tokens are also shown because
  subscription accounting differs by provider. Forge wins raw and 25%-adjusted totals here, while
  both native harnesses use fewer tokens when cached input is treated as completely free.
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
