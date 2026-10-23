# Feature: progress supervision, with no default step cap

> Status: **SHIPPED** (direct tool loop). Code: `crates/forge-core/src/progress_ledger.rs` (the
> judgement), `progress_report.rs` (what the model and the person are told),
> `progress_supervisor.rs` (the session side), `loop_progress.rs` (what one call looked like),
> `stall_guard.rs` (the narration lane), `turn_guards.rs` (explicit limits and spend ceilings),
> `crates/forge-config/src/progress_guard.rs`, `hard_stop.rs`. Bridges: `bridge_progress.rs` and
> `crates/forge-cli/src/mcp_serve/progress.rs`. Tests: `progress_ledger/tests.rs`,
> `tests/progress_supervision.rs`, `tests/fixtures/bench_replays.json`.

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
- on an unattended turn, a spend ceiling is hit.

**Step count is never a default limit.** `mesh.max_steps` and `mesh.max_steps_unattended` default
to `0` (none). Every stop says why and how to continue; `continue` is an ordinary prompt.

## 3. The ledger

One `ProgressLedger` per session lives for the whole turn, across every re-drive pass. After each
tool step it classifies what the step produced:

| Class | Condition | Effect |
|---|---|---|
| Strong | a successful file write whose arguments were not tried this turn; a tracked task moved to Done during the step; a failing check (test, lint, typecheck, build) passed; the person steered | resets every lane and the episode |
| Informative | at least a quarter of the result's lines were not seen this turn | breaks a run of futile steps only |
| Neutral | every call was a poll-mode wait that came back "not ready"; or the first step after a harness gate (a verify, completeness or continue re-drive) | counts nowhere |
| All-failed | at least one call and every call failed | stale evidence |
| Stale | anything else | stale evidence |

A compaction makes the ledger forget what was seen (the model lost it, so reading it again is
legitimate) but keeps the counters and any pending nudge.

Shell-driven work (a silent `sed -i`, a `git commit`) looks stale to the ledger. Each nudge records
the working-tree fingerprint, and before any stop it is taken again: if it differs, that counts as
Strong progress, the episode starts over, and the turn is not stopped. Outside a git repository
there is nothing to compare and no veto.

### Lanes

Five lanes watch the stale evidence under one latch.

| Lane | Counts | Nudge | Stop |
|---|---|---|---|
| Narration | restated openings on stale steps (`NarrationTracker`, window of 10) | 3 in the window | 2 relapse steps in the 4 after the nudge |
| Futile | consecutive stale or all-failed steps | 5 | 10 |
| Failure | consecutive all-failed steps on the same tool, no success between | 3 | 6 |
| Cycle | the same batch of calls with the same result 3 times in a row, or at most two distinct batches in the last 6 with one recurring 3 times and identical results | when it holds | after 2 further repeats |
| Idle | steps since the last Strong step | `mesh.idle_nudge_steps` (60), on every surface | `mesh.idle_stop_steps` (200), on unattended turns only |

Rules that tie them together:

- **One latch.** One nudge per episode, whichever lane fires first.
- **A stop needs a nudge first**, at least two steps after it, and then the lane's own stop bar.
- **An episode closes** on a Strong step, or after four consecutive steps with every lane under its
  nudge line.
- **Idle is generous on purpose.** It is the only lane that can end a read-only turn of novel
  output, so its bars sit well above anything healthy that was measured (the longest stretch
  without an edit in a solved task was 37 steps). An attended turn is never stopped for idleness:
  the person can see it and press escape.

### After a stop

- The ledger remembers what the stop was on (the call and the opening sentence). If one of the
  first four steps after a `continue` repeats either, the turn is stopped again at once with no
  second nudge. This is the "you are getting stuck" typed three times, handled by the harness.
- A system note, `Forge stopped your previous turn (<lane>: <reason>). Do not resume the same
  calls`, is stored so the next turn's model knows why it was stopped.
- A re-drive cannot restart a stalled turn: `hard_guard_short_circuit` refuses to enter the loop
  again, so a blocking Stop hook or the empty-diff push-back cannot reset the evidence that
  stopped it.

### Harness notes do not talk to themselves

`push_note` drops the fourth identical copy (same kind, same text) of a `Hint`, `LoopGuard`,
`Continue` or `Silence` note within a turn. A harness that repeats a hint every step is feeding the
model the pattern it then repeats; this is also what keeps an LSP-style runaway, whose writes count
as Strong progress and so are invisible to the lanes, from filling the context with one notice.

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

On an unattended turn (headless `forge run`, JSON and stream-json output, bench, `--mode bypass`)
the same headline is an error with the uncommitted work preserved
(`refs/forge/aborted-turns`) and `Resume: forge run --continue`; the process exits non-zero.

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
| `mesh.idle_nudge_steps`, `mesh.idle_stop_steps` | the idle lane's bars; `0` disables each |

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
- A stop is vetoed when the working tree changed since the nudge, and `progress_guard = "warn"` or
  `"off"` never refuses a call, as on the direct path.

The bridge keeps its re-drive budget (it now counts re-drives that close no task), the 300 s idle
watchdog and the per-turn bridge token ceiling. The text-mode and `agy` bridges have no tool hook
in `mcp-serve` and stay protected by those alone.

## 8. Not covered yet

- **MCP `forge_chat` and daemon workers in Default or AcceptEdits mode** count as attended, so they
  get nudges and lane stops but no idle stop.
- The thresholds are priors from a small sample of real runaways. Run with
  `progress_guard = "warn"` for a while before trusting the stops on your own work.
