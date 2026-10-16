# Recall baseline, 2026-10-16

How much of a long session survives compaction and a resume, measured on the behavior before
the compaction rework (compaction exactly as it stood on main: summarizer chosen by the
existing candidate chain, tool results pruned before summarizing, no rehydration). Two
scenarios: **standard** (23 turns, near the ceiling for a capable model) and **hard** (38 turns
under a 24k auto-compaction ceiling, the one that reproduces the forgetting). Read the hard
section first.

| Scenario | After compaction (live session) | After resume (isolated) |
|---|---:|---:|
| standard | 17/18 94.4% (see the method caveat) | 17/18 94.4% |
| **hard** | **12/24 50.0%** | **12/24 50.0%** |

## Hard scenario

| | |
|---|---|
| Date | 2026-10-16 |
| Forge code | `97b3f11d`, unchanged; only the bench was added |
| Model | `inferhub::glm-5.3-flash`, pinned, with `[compact_cap]` of 24,000 tokens |
| Command | `forge bench recall --scenario hard --json --keep-workdir --price-in 0.15 --price-out 0.5` |
| Scenario | 38 turns, 23 probes scored as 24 results, 5 auto + 2 manual compactions, probes after compaction and after resume |
| Raw result | [recall-baseline-hard-2026-10.json](artifacts/recall-baseline-hard-2026-10.json) |

**Compactions during the scenario: 5 automatic, 2 manual (7 in all), none while probing.**
Each wrote a summary:

| # | Kind | Messages | Summary written by |
|---|---|---|---|
| 1 | auto | 106 -> 7 | `gemini::gemini-2.5-flash` |
| 2 | auto | 42 -> 7 | `gemini::gemini-2.5-flash` |
| 3 | auto | 25 -> 7 | `gemini::gemini-2.5-flash` |
| 4 | manual (mid-scenario) | 24 -> 20 | `gemini::gemini-2.5-flash` |
| 5 | auto | 170 -> 7 | `gemini::gemini-2.5-flash` |
| 6 | auto | 60 -> 10 | `inferhub::glm-5.3-flash` |
| 7 | manual (near the end) | 33 -> 7 | `inferhub::glm-5.3-flash` |

Five of seven summaries were written by a free Gemini model, not by the session's model, which
matches the plan's finding that the chain tries the best free capable model first; the session's
model wrote the last two, and why was not recorded. No summary was rejected as too thin.

| Category | After compaction | After resume |
|---|---:|---:|
| constraint | 0/2 | 0/2 |
| correction | 1/2 | 1/2 |
| decision | 1/2 | 1/2 |
| location | 1/2 | 1/2 |
| error_fix | 1/2 | 1/2 |
| pending | 3/3 | 2/3 |
| number | 1/3 | 1/3 |
| behavior (work tasks) | 1/3 | 2/3 |
| tool_result (value only in a tool result) | 1/2 | 1/2 |
| work_state | 1/2 | 1/2 |
| task_list | 1/1 | 1/1 |
| **Total** | **12/24 50.0%** | **12/24 50.0%** |
| early facts (turns 1-11) | 4/14 | 4/14 |
| tool-output facts (turns 12-13) | 1/2 | 1/2 |
| late facts (turns 23-28, after compaction #4) | 7/8 | 7/8 |

Per probe (pass / fail, after compaction then after resume):

| Probe | Category | Compaction | Resume |
|---|---|:-:|:-:|
| commit_prefix (JIRA-123, turn 1) | constraint | fail | fail |
| data_readonly (never edit data/, and why) | constraint | fail | fail |
| backoff_decision | decision | fail | fail |
| added_function (retry_budget in limits.rs) | location | fail | fail |
| error_code (E-4172) | error_fix | fail | fail |
| error_fix_value (16384) | error_fix | pass | pass |
| staging_port (47123) | number | fail | fail |
| latency_target (230 ms) | number | fail | fail |
| pending_test | pending | pass | fail |
| pending_version (0.4.2) | pending | pass | pass |
| config_filename (settings.toml, the correction) | correction | fail | fail |
| region_quota (7311, only in a file read) | tool_result | fail | fail |
| check_output (BEEF42, only in command output) | tool_result | pass | pass |
| indent_override (tabs replace spaces) | correction | pass | pass |
| tasks_open | task_list | pass | pass |
| pending_changelog | pending | pass | pass |
| rollout_window (03:15) | number | pass | pass |
| ascii_decision | decision | pass | pass |
| renamed_helper | location | pass | pass |
| work_state_files | work_state | fail | fail |
| work_state_pending | work_state | pass | pass |
| obey_agents_doc_rule (AGENTS.md `Forge:` doc comments) | behavior | fail | pass |
| obey_latest_indent (tabs, the override) | behavior | pass | pass |
| obey_commit_prefix | behavior | fail | fail |

What it shows:

- **What was said early is gone, what was said late is not.** 4 of 14 early facts survive, 7 of 8
  late ones do. The late facts were planted after compaction #4 and survived three more
  compactions; the early ones went through all seven. It is not a size effect: the early
  instructions are the user's own words (a commit prefix, a protected directory, a port, a file
  name correction) and the summary dropped them.
- **The model then denies them.** Typical answers: "Our conversation never established a
  repo-specific commit prefix", "You never mentioned a staging server or a port", "No region table
  was ever read". The work probe confirms the cost: asked to commit, it used `fix: ...` in both
  phases, breaking the JIRA-123 rule. A fact the summary drops is not flagged as missing; the model
  is confident it never existed.
- **The AGENTS.md rule is lost after compaction in the live session** (the new function had no
  doc comment starting `Forge:`) **and followed after resume**; the cause of the difference was
  not isolated. The tab override (a late instruction) was followed in both.
- **Values that only existed in a tool result mostly vanish**: the quota on row 233 of the table
  was never recalled. The checksum from the command output survived only because the model quoted
  that line in its own reply and later answers kept repeating it.
- **The work-state probe**: pending items were named (3/3 in both phases), but the file list was
  incomplete in both phases (the model left out `src/parser.rs`, and after resume `src/limits.rs`
  too), scored against every file that differs from the baseline commit, committed or not.
- **Resume does not change the total** but moves individual probes (`pending_test` and the
  AGENTS.md rule trade places). A different kept tail after resume is one candidate; this run
  does not separate it from run-to-run variation.

Spend: 3,376,328 input tokens (2,842,740 cached), 48,196 output (19,103 reasoning), 291 model
calls, 122 tool calls, cost $0.0711, wall 2,486 s. Cost is the store's figure at the same
estimated price as the standard run ($0.15 / $0.50 per million tokens) for the pinned model; the
Gemini summaries are in the token counts but may be recorded at zero.

### Method and its limits

- **Two phases, different isolation.** After-compaction probes run in sequence on the live
  session, so an earlier answer can leak into a later one; after-resume probes each get their own
  session resumed from a checkpoint of the store taken right after the scenario. The two totals
  agree here (12/24), so leakage did not decide the result.
- **Why not `/undo` between probes.** The first harness version rewound each probe with `/undo`.
  In that run an auto-compaction of 380 messages (`380 -> 24`) fired during the first probes,
  right after the first rewind, in a session whose last compaction had left 9. That matches the
  plan's finding that the pre-rework reload after `/undo` resurrects the full history, so every
  later probe measured one fresh summary of the whole history instead of the scenario's own
  compactions. The discarded run scored 19/24 and 20/24 (79% and 83%) against 12/24 here, so a
  single summary pass over full-fidelity history kept far more than seven progressive ones;
  that is a hypothesis this bench can now test, not a result.
- **A first launch was also discarded.** Told to "read" files, the model answered with
  `wc`/`grep` one-liners, so nothing large entered the context and no compaction fired. The
  prompts now require whole-file `read_file` calls.
- **The work-state file check uses `git diff` against the baseline commit.** The model commits
  on its own, so a check against the working tree alone sees almost nothing.
- **One run, one model, one seed.** A model's answers vary; individual probes can flip on a
  rerun. The size of the gap (early 4/14 against late 7/8) is the signal, not any single probe.
- **The shell runs through the user's `rtk` output filter**, as in a daily session, which shrinks
  some command outputs. The kept store of the run is not available (it was only added to
  `--keep-workdir` afterwards).

## Standard scenario

| | |
|---|---|
| Date | 2026-10-16 |
| Forge code | `97b3f11d` (origin/main when the branch was cut); the bench itself is the only addition |
| Model | `inferhub::glm-5.3-flash`, pinned |
| Command | `forge bench recall --json --keep-workdir --price-in 0.15 --price-out 0.5` |
| Scenario | 23 scripted turns, 18 probes (16 questions, 2 behavior), 2 compactions, 1 resume |
| Raw result | [recall-baseline-2026-10.json](artifacts/recall-baseline-2026-10.json) |

The harness is described in [bench-recall.md](../features/bench-recall.md).

### Result

| Category | After compaction | After resume |
|---|---:|---:|
| constraint | 3/3 100.0% | 3/3 100.0% |
| correction | 1/1 100.0% | 1/1 100.0% |
| decision | 3/3 100.0% | 3/3 100.0% |
| location | 2/2 100.0% | 2/2 100.0% |
| error_fix | 2/2 100.0% | 2/2 100.0% |
| pending | 2/2 100.0% | 2/2 100.0% |
| number | 2/3 66.7% | 2/3 66.7% |
| behavior | 2/2 100.0% | 2/2 100.0% |
| **Total** | **17/18 94.4%** | **17/18 94.4%** |
| early facts (planted before compaction #1) | 15/15 100.0% | 15/15 100.0% |
| late facts (planted between the compactions) | 2/3 66.7% | 2/3 66.7% |

Compactions: mid-scenario 134 -> 21 messages, near-end 52 -> 15 messages. No scenario turn
failed. No memory question used a tool (the scores are not inflated by re-reading files).

The one miss is `late_bench_budget` ("the header benchmark must stay under 12 ms for 10,000
headers"), a number planted after the first compaction and folded by the second. Both phases
answered "you never gave a time budget", so the summary dropped it, not the resume. Both behavior
probes passed in both phases: the new function in `src/parser.rs` had no `.unwrap()`, and the
commit subject started with `JIRA-123`.

### Spend

| | |
|---|---:|
| Input tokens | 1,316,543 (1,151,374 cached) |
| Output tokens | 9,733 (2,972 reasoning) |
| Model calls / tool calls | 160 / 39 |
| Cost | $0.0069 |
| Wall time | 1,052 s |

The catalog has no price for this model, so cost is computed from `--price-in 0.15` and
`--price-out 0.5` USD per million tokens, the rate an earlier Forge config recorded for
`glm-5.3-flash`. Treat the dollar figure as an estimate; the token counts are the store's.

### What this does and does not show

- **This is a ceiling, not a stress test, for this model.** At 23 turns and a 1M-token window the
  session never got large, so a capable model's own summary kept every early fact. Recall only
  drops on the late number, where the second compaction summarized a summary. Headroom to show an
  improvement is therefore small: `--turns-scale 3` or later-planted facts are the way to widen it.
- **Method caveat.** This run rewound each probe with `/undo`, which on this code reloads the
  whole uncompacted history after a compaction (see the hard scenario's method notes). Only the
  first after-compaction probe is certain to have seen the compacted state, so read the
  after-resume column (each probe on the persisted state, no compaction fired during probing
  because no ceiling was set) as the valid one; the two columns are identical. The bench has
  since been changed to isolate probes without `/undo`; the standard scenario has not been rerun
  with it.
- **One run, one model.** Replies are stochastic; a single miss of 18 is within what a rerun can
  change. Use the per-category rows as a guide and rerun before drawing a conclusion from one fact.
- **Config differs from daily use.** The bench runs with its own generated config, not the real
  one: no user system-prompt override, no code index, no discovery catalog, no self-MCP, and only
  the pinned model's provider, context window (1,000,000) and price copied over. With no discovery
  catalog the summarizer chain may differ from a real install's (which can shortlist other
  models), so the summary quality here is not necessarily what a daily session gets.
- **Facts are checked by regex.** The matchers accept any phrasing containing the right file name,
  value or reason keyword; they cannot tell whether the model also kept the surrounding nuance.

Reproduce with the command above (needs an InferHub key in the Forge key store); `--mock` checks
the plumbing without a model.
