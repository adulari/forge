# Feature: progress supervision, with no default step cap

> Status: **SHIPPED** (direct tool loop). Code: `crates/forge-core/src/progress_ledger.rs` (the
> judgement) with `progress_ledger/tree.rs` (working-tree states) and `check.rs` (checks across a
> turn), `progress_report.rs` (what the model and the person are told),
> `progress_supervisor.rs` (the session side), `loop_progress.rs` (what one call looked like;
> `loop_progress/waiting.rs` and `text.rs`),
> `stall_guard.rs` (the narration lane), `turn_guards.rs` (explicit limits and spend ceilings),
> `crates/forge-config/src/progress_guard.rs`, `hard_stop.rs`. Bridges: `bridge_progress.rs` and
> `crates/forge-cli/src/mcp_serve/progress.rs`. Tests: `progress_ledger/tests.rs`,
> `progress_ledger/regressions.rs`, `tests/progress_supervision.rs`,
> `tests/fixtures/bench_replays.json`.

## 1. Problem

Forge stopped, or interrupted, turns by counting steps: 100 model calls paused an attended turn,
and an unattended one printed

```
reached soft step cap 100 at step 100; unattended turn continuing toward hard cap 400
(turn tokens: billable input=1176241, total input=7529335 incl. cache reads, output=44272)
```

and ended at 400. That is the wrong unit.

- The count says nothing about whether the turn is getting anywhere. A hundred steps that edit,
  test and close tasks are a healthy turn; ten steps that re-read one file are not. In the message
  above the turn had used 12% of its token ceiling and nothing was wrong.
- It bounds nothing that costs money. A 400-step turn reached 48.5M tokens (an LSP retry loop
  feeding hints into the context), and a 100-step turn burned 7.0M.
- It was applied per `run_model_loop` pass, so "100 steps" was not even a turn limit: each re-drive
  (completeness pass, verify gate, empty-diff push-back, stop-hook continuation) started again at
  zero.
- Claude Code and Codex have no such cap in interactive use. A turn ends when the model stops, the
  person interrupts, or context management kicks in.

Under the count sat five separate detectors (identical-call doom loop, oscillation, failure loop,
structural stagnation, narration) with their own thresholds, latches and nudge texts. One spiral
could collect several nudges and several halts, and none of them knew about a harness re-drive.

## 2. The contract

A turn ends only when:

- the model finishes;
- the person interrupts;
- the progress supervisor decides it is stuck: one nudge, then a clean stop;
- an explicit limit the user set is reached (`mesh.max_steps`, `--max-turns`, `--max-budget-usd`,
  goal and loop budgets);
- on a turn nobody is watching, a spend ceiling is hit.

**Step count is never a default limit.** `mesh.max_steps` and `mesh.max_steps_unattended` default
to `0` (none). Every stop says why and how to continue; `continue` is an ordinary prompt.

## 3. The ledger

One `ProgressLedger` per session lives for the whole turn, across every re-drive pass. After each
tool step it classifies what the step produced:

| Class | Condition | Effect |
|---|---|---|
| Strong | a successful file write whose arguments were not tried this turn; a tracked task moved to Done during the step; a failing check (test, lint, typecheck, build) passed, the first time that failure was fixed; the working tree reached a state it was never in this turn; the person steered | resets every lane and the episode, except a check's run of identical failures (below) |
| Informative | at least a quarter of the result's lines were not seen this turn; a path read for the first time; a silent shell command with arguments not tried this turn | breaks a run of futile steps only |
| Neutral | a wait for something outside the turn (below); or the first step after a harness gate (a verify, completeness or continue re-drive) | counts nowhere, except that waits are counted by the waiting lane |
| All-failed | at least one call and every call failed | stale evidence |
| Stale | anything else | stale evidence |

A compaction makes the ledger forget what was seen (the model lost it, so reading it again is
legitimate, and a note it was told is worth telling again) but keeps the counters and any pending
nudge.

**Waits.** The harness tells the model to wait for a background job, a CI run or a server by
polling, and a poll returns the same answer until the thing is ready. These calls are neutral:
`shell` in poll mode that came back "not ready", `shell` cut by its time limit, a successful
`shell` whose command sleeps or watches first (`sleep 60; gh run view`, `gh run watch`), and
`shell_job` status or log of a job that is still running. A step with a wait in it is a wait step
even if it also peeked at a log. How long a turn may wait is the waiting lane's business.

**The working tree.** Shell-driven work (a silent `sed -i`, a `git commit`, a generator script)
returns `exit 0` every time, so a ledger that only reads results sees nothing. After a step whose
calls could have changed files unseen (anything but a read, a search or a task-list update) the
tree is sampled: in a git repository HEAD, status and diff plus the content of untracked files,
outside one a bounded scan of the directory's files by content. A state no earlier sample was in
is Strong progress. A state seen before is not, so a file that flips between two or five contents
is in a known state from its second visit on, however often it moves. A change the model made
with a write tool is not credited to the shell step that follows it (that step only moves the
baseline), a sample slower than 0.4 s makes the next eight skip, and a tree that cannot be read is
no evidence either way.

**Checks.** A check (test, lint, typecheck, build) is judged by its verdict, not by the edits
around it. A flip from failing to passing is Strong only the first time that failure (its output
without durations, times and ids) is fixed; breaking and repairing the same thing each round is
not progress each time it passes. And a check that fails with the same output run after run is
not listening to the edits between the runs, which is the check lane below.

### Lanes

Seven lanes watch the evidence under one latch.

| Lane | Counts | Nudge | Stop |
|---|---|---|---|
| Narration | restated openings on stale steps (`NarrationTracker`, window of 10) | 3 in the window | 2 relapsing steps in the 4 after the nudge: stale, or the same opening over output that only looked new (a step that read a path for the first time is not one) |
| Futile | consecutive stale or all-failed steps | 5 | 10 |
| Failure | consecutive steps in which every call failed with the same error (digits ignored), unless a failure prints three or more lines a quarter of which are new (failing tests, compiler output: the model is learning something); a failure with no message (a search that found nothing) is not one and falls to the futile lane | 3 | 6 |
| Cycle | the same batch of calls with the same result 3 times in a row, or at most two distinct batches in the last 6, one recurring 3 times, each with one result throughout | when it holds | after 2 further repeats |
| Check | runs of one check failing with the same output, however much was edited between them | 6 | 12 |
| Waiting | wait steps since the last Strong step | 24, on every surface | 96, on headless turns only |
| Idle | steps since the last Strong step | `mesh.idle_nudge_steps` (60), on every surface | `mesh.idle_stop_steps` (200), on headless turns only |

Rules that tie them together:

- **One latch.** One nudge per episode, whichever lane fires first.
- **A stop needs a nudge first**, at least two steps after it, and then the lane's own stop bar.
- **An episode closes** on a Strong step, or after four consecutive steps with every lane under its
  nudge line. The check lane's episode is the exception: edits do not end it, only the check
  passing or failing differently does.
- **Headless means no person is watching**: a headless `forge run`, JSON and stream-json output,
  bench. `--mode bypass` in a terminal someone is looking at is not headless, so the person gets
  the warning and the status, not an error, and no idle or waiting stop. `mesh.idle_stop_steps = 0`
  turns off both headless stops.
- **Idle and waiting are generous on purpose.** They are the only lanes that can end a turn whose
  every step looks new or is a poll, so their bars sit well above anything healthy that was
  measured (the longest stretch without an edit in a solved task was 37 steps). A person watching
  can press escape.
- **A subagent has no idle or waiting stop of its own**: whether anyone watches it is the
  parent's business, and it is held to the other lanes and to its agent type's `maxTurns`.

### After a stop

- The ledger remembers what the stop was on (the call and its result, and the opening sentence). If
  one of the first four steps after a `continue` repeats the call and it comes back the same, or
  repeats the opening over nothing new, the turn is stopped again at once with no second nudge.
  This is the "you are getting stuck" typed three times, handled by the harness. The same call
  that now comes back differently (the person started the service it was probing) is the
  `continue` working and is left alone.
- A system note, `Forge stopped your previous turn (<lane>: <reason>). Do not resume the same
  calls`, is stored so the next turn's model knows why it was stopped.
- A re-drive cannot restart a stalled turn: `hard_guard_short_circuit` refuses to enter the loop
  again, so a blocking Stop hook or the empty-diff push-back cannot reset the evidence that
  stopped it.

### Harness notes do not talk to themselves

`push_note` drops the fourth identical copy (same kind, same text) of a `Hint`, `LoopGuard`,
`Continue` or `Silence` note in a burst. A harness that repeats a hint every step is feeding the
model the pattern it then repeats; this is also what keeps an LSP-style runaway, whose writes count
as Strong progress and so are invisible to the lanes, from filling the context with one notice.
A copy that comes 25 steps after the last one, or after a compaction (the model no longer has the
earlier copies), is admitted again, so a diagnostic that persists is not lost for the rest of the
turn.

## 4. What the person sees

The nudge goes to the model once per episode, as a `LoopGuard` note, and names observations:

```
Progress check from Forge. In your last 4 steps nothing changed and nothing new turned up.
Observed: `shell` failed 4 times (command not found: x); `read_file a.rs` ran 3 times with
identical results; the `test` check is still failing; 2 tasks open.
Last real progress: step 31 (edit_file src/lib.rs).
Either fix the cause, try a different approach, answer from what you have and say what is
uncertain, or, if waiting on something external, say so and stop polling.
```

The idle nudge is gentler ("N steps without changing a file or closing a task. If this is an
investigation, report what you found and what you still need; otherwise make the change.").
`harness_profile = "minimal"` sends no nudge text and keeps the same stop schedule.

A stop on an attended turn is a warning, not an error, and the model writes a short status:

```
Stopped: no progress for 10 steps (last change: step 31, edit_file src/lib.rs). Your edits are kept.
Send `continue`, or tell it what to try.
```

On a headless turn (`forge run`, JSON and stream-json output, bench; a person at a terminal in
`--mode bypass` still counts as watching) the same headline is an error with the uncommitted work
preserved (`refs/forge/aborted-turns`) and `Resume: forge run --continue`; the process exits
non-zero.

The turn ends with `StopReason::Stalled` (wire `stalled`, outcome `failed`). The TUI shows
"Paused: no progress" with the way on; the board shows "stalled, no progress"; the JSON result keeps
`error_during_execution`.

## 5. Limits that are still there, all explicit

| Setting | Meaning |
|---|---|
| `mesh.max_steps = N` | a hard cap on model calls in one turn (`0` = none, the default). Attended turns pause with a warning and `continue` resumes; unattended turns fail |
| `mesh.max_steps_unattended = N` | the same for unattended turns; wins over `max_steps` there |
| `--max-turns N` | sets both for one run (JSON subtype `error_max_turns`) |
| `--max-budget-usd`, `/goal` and `/loop` budgets | unchanged; they end the turn with `budget_exhausted` |
| `mesh.max_turn_input_tokens`, `mesh.bridge_turn_token_cap`, the USD turn cap | stop an unattended turn as `budget_exhausted` (a token-ceiling or dollar-cap abort used to be reported as a finished answer); an attended turn gets one warning, see `mesh.budget.hard_stop` below |
| `mesh.budget.hard_stop = true \| false \| "auto"` | `auto` (the default) stops unattended turns at those ceilings and only warns attended ones; `true` stops every turn, `false` relaxes the USD caps only. The session, day, week and month caps are enforced either way |
| `mesh.progress_guard = "enforce" \| "warn" \| "off"` | `warn` turns every stop into "would have stopped here" and lets the turn continue (how to calibrate on real work); `off` keeps the nudges and never stops |
| `mesh.idle_nudge_steps`, `mesh.idle_stop_steps` | the idle lane's bars; `0` disables each. The stop applies to headless turns only, and `idle_stop_steps = 0` also turns off the headless waiting stop |

"Unattended" in the spend rows is the older notion (headless, or `--mode bypass`, where no prompt
can be answered); the idle and waiting stops are narrower and ask only whether a person is
watching.

An explicit step limit is counted against the turn's model calls across every re-drive pass.
Subagents use the same rule: `maxTurns` is capped by `mesh.max_steps` only when that is set, and a
child that stops getting anywhere is stopped by its own ledger and hands its last words and the
reason back to the parent instead of an error that throws its work away.

A config that still pins the old defaults (`max_steps = 100` together with
`max_steps_unattended = 400`) is reported once at startup: both are hard limits now, so removing
the two lines is what selects progress-based stopping.

## 6. What was removed

The attended 100-step pause and the 400-step unattended ceiling as defaults; the soft-cap warning;
the per-pass guard locals in `run_model_loop`; the identical-call doom and oscillation guard; the
failure-loop guard; `ToolFailureTracker` (which also refused the third identical call outright);
the `ProgressTracker` verdict ladder; and the silence guard's 40-step "summarize your findings"
ask (the idle lane replaces it). Kept as components: `NarrationTracker`, the line fingerprints, the
silence guard's progress line, and every content gate (verify, phantom edit, completeness).

Three corrections found on the way:

- `shell` with `"commands": []` next to `"command"` was rejected as contradictory; it is now "no
  batch". That one mistake produced 15 of the 20 failure nudges in the measured corpus (193
  rejected calls across 60 sessions).
- A silent shell command's whole result is the header `shell: exit 0 in 42ms`; its timing made
  every run look like new output, so a loop of silent commands was never stale. The header's digits
  are masked.
- The bridge re-drive budget (8) counted every re-drive; it now counts re-drives that close no
  task.

## 7. CLI bridges

A bridged model (`claude-cli`, `codex-cli`) runs its own tool loop in a subprocess, so the parent
session sees one response per run and none of the calls. Every call does land in `forge mcp-serve`
with its full result, so the same ledger runs there, call by call (`BridgeProgress`), with the
narration and idle lanes off and the futile bars doubled (nudge at 10, stop at 20) because one call
is one step. Cycle (3 and 5) and failure (3 and 6) are unchanged.

- A nudge is put in front of the result of the call that earned it, so the model reads it in-band.
- A stop replaces that result, and every later call's, with `Forge stopped this turn: ... Make no
  more tool calls; reply now with a short summary of what you found and what is still open.`
- `mcp-serve` also reports the stop through the sink (`{"k":"stall"}`), so the parent ends the turn
  as `stalled` with the model's summary as the answer instead of re-driving a plan that is not
  moving, and stores the same "Forge stopped your previous turn" note.
- A model that keeps starting tools more than 30 s after the stop has its process group killed;
  the turn still ends as stalled, and that call's usage is lost.
- Each call that could have changed files is followed by a look at the working tree, as on the
  direct path (the first call of a turn has no earlier state to compare with). `progress_guard =
  "warn"` or `"off"` never refuses a call.
- The waiting lane nudges at 48 calls (one call is one step) and never stops a bridged turn: a
  bridge has no way to know whether anyone is watching.

The bridge keeps its re-drive budget (it now counts re-drives that close no task), the 300 s idle
watchdog and the per-turn bridge token ceiling. The text-mode and `agy` bridges have no tool hook
in `mcp-serve` and stay protected by those alone.

## 8. Known limits

- **A loop in which some step's output always changes is caught only by the idle lane.** A step
  that prints a timestamp, a counter or a fresh log line every time looks new, so reading file a,
  running `date`, reading file b, forever, is nudged at 60 steps and stopped only on a headless
  turn at 200. The same holds for repeating one sentence over a novel read every step. Text
  cannot tell noise from news; the person's escape key and the spend ceilings are the bound.
- **A headless read-only audit of more than 200 files** with no edit, no closed task and no check
  is stopped at 200 with a way to resume. Close tasks as you go, or set `idle_stop_steps = 0`.
- **A compaction every few steps hides a re-read loop** from the stale lanes (the model forgot, so
  each re-read is new); the idle nudge at 60 still fires.
- **A process that keeps writing inside the workspace** (a dev server logging to an untracked
  file) moves the tree on every sample, so every shell step looks like progress while it runs.
- **Shell work that leaves no trace in the tree and prints nothing** (a restart, an install
  outside the workspace) counts as a new action only while its arguments are new.
- **A quiet hand-rolled poll without a sleep** (`tail -n 1 build.log` five times in a row, no pause
  between) is a loop and is stopped; the same poll with `sleep 30;` in front of it, or through
  poll mode or `shell_job`, is a wait.
- **The first sample of a bridged turn** has nothing to compare with, so a bridged turn's first
  silent shell command is judged by its arguments alone.
- **MCP `forge_chat` and daemon workers in Default or AcceptEdits mode** count as attended, so they
  get nudges and lane stops but no idle or waiting stop.
- The thresholds are priors from a small sample of real runaways. Run with
  `progress_guard = "warn"` for a while before trusting the stops on your own work.

## 9. Measured

Six tasks from suite2 chosen for long turns and guard history (`b01-stockctl-longhorizon`,
`a02-sliding-window-limiter`, `a04-debug-nightly-export-crash`, `a05-mailer-options-object`,
`b07-ledger-rollup-awk`, `d04-long-build-await`), the native loop on
`inferhub::cb/deepseek-v4.1-flash`, headless, one attempt per run:

| Binary | Runs | Solved per run | Model calls per run | Tool calls per run | Agent wall per run | Tokens per run |
|---|---|---|---|---|---|---|
| before (`forge-after1-734e482d`) | 3 | 4, 5, 4 | 117, 134, 125 | 249, 286, 269 | 913, 1074, 1576 s | 2.65M, 3.43M, 3.41M |
| first ledger (`edd56f4c`, then `ad3bc5b3`) | 3 | 4, 4, 4 | 132, 103, 118 | 265, 218, 254 | 1287, 1218, 1151 s | 3.12M, 1.96M, 2.68M |
| this version (`1bb97cf8`) | 2 | 5, 5 | 106, 123 | 245, 266 | 1029, 1019 s | 2.26M, 2.58M |

Run-to-run spread from the model is larger than the difference between the binaries: `a04` fails
in all 8 runs of every binary, and `a05` is solved in 1 of the 3 earliest runs, 0 of 3 with the
first ledger and 2 of 2 now, which is the model's variance and not something the ledger did. So
this shows no regression, not an improvement. No turn came near a step cap, so the headline
result is about events:

- **Ledger events in the 30 task runs of the new binaries: none.** No nudge, no stop. The runs did
  include harness gates (the existing-tests push-back in `a05` and `b01`, and the "latest test
  check failed" continue gate twice in one `b01`, a 41-call turn); those steps are neutral.
- **The old guards in 18 task runs: one false alarm.** In `a04` the failure tracker refused the
  model's third identical `read_file` of the same CSV outright, from inside a `run_code` script
  (`tools.read_file(...) failed: doom-loop`), although the file had been edited in between.
  `ToolFailureTracker` is gone and a repeat now has to return an identical result three times.

The bench never loops, so the cases the ledger is for are measured against the real binary with a
scripted zero-quota stub model behind an OpenAI-compatible endpoint (headless, defaults;
`runner2/live_stub.sh` in the bench kit):

| Scenario | Before (`forge-after1-734e482d`) | Now |
|---|---|---|
| the same `read_file` forever | halted after 4 calls, exit 0 | nudge at step 3, stop at 5, `ERROR: Stopped: the same calls returning the same results for 5 steps ... Resume: forge run --continue`, exit 1 |
| 40 polls of a running background job (`shell_job status`) | halted after 5 calls | finishes, nothing printed |
| 120 such polls | | headless: stop at 96, `still waiting for something that is not ready`, exit 1; attended: one nudge at 24, no stop |
| 40 distinct silent shell commands, each writing a new file | halted after 12 calls | finishes |
| 8 failing scripts, each printing different output | halted after 6 calls | finishes |
| a tracked file rewritten through 3 states, 60 steps | halted after 26 calls | stopped after 13 calls |
| 30 different edits, each followed by the same failing test | ran all 61 calls | nudge at 12, stop at 24 (`the same check failing the same way after every edit (12 runs)`) |
| 250 novel reads | ran all 251 calls | headless: stop at 200; attended: finishes |

`forge mcp-serve` driven over stdio the way a bridged model would: the third identical call returns
the nudge in front of its result, the fifth returns the refusal, every later call is refused, the
sink gets `{"k":"stall"}`, and the same call in the next turn is refused at once.
