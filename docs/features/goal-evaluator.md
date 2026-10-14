# Feature: `/goal` progress evaluator

> **Status: shipped.** On by default. `[goal] evaluator = false` restores the fixed continue
> prompt.

## 1. Problem (JTBD)
> When I leave `/goal` running, I want each turn to know what is actually still missing, and the
> run to stop when the goal is met or plainly stuck — not keep re-reading "continue the goal"
> while it guesses what is left from its own task list.

Before this, every goal turn after the first got the same static `GOAL_CONTINUE_PROMPT`.
Completion was inferred from the model's own `update_tasks` plan (all tasks done) or an exact
`GOAL COMPLETE` reply, and a stall was "six turns without a task ticked off". The model never got
feedback grounded in what it had actually done.

## 2. Behaviour

After each goal turn, one cheap side call — the **goal evaluator** — is shown:

- the goal text and the turn number;
- the tracked task plan with statuses;
- the turn's final answer (its last 3,000 characters);
- evidence read from the workspace, not from the model: commits since the goal started
  (`git log --since`), a diff stat against the last commit before the goal started (committed and
  uncommitted work), `git status --short`, and the passing verification commands the
  [evidence ledger](evidence-ledger.md) recorded for the session;
- the last quality-gate failure, if any, and the previous verdict's remaining list.

It replies with JSON: `achieved`, `remaining` (at most six short concrete items), `next_step`,
`confidence`, `progressed`, and a one-line `reason`. The parser tolerates code fences, prose
around the object, string booleans, a string list and percentages.

Each verdict is shown as a compact line, through the session presenter as a `Progress` event, so
it reaches the TUI scrollback, headless output and the remote snapshot the phone and browser
render:

```
⋯ 🎯 2 left · next: add the `done` subcommand to todo.py
⋯ 🎯 goal achieved: add, list and done exist and the tests pass
```

What the loop does with it:

| Verdict | Next step |
| --- | --- |
| achieved (confidence ≥ 0.5, or the agent also says it is done) | run the `--gate` quality gates; stop with the normal completion if they pass, re-drive with the gate output if one fails |
| not achieved | re-drive with a short prompt built from the verdict: the goal, the evaluator's reason, what is still missing, the next step, and the last gate failure |
| not achieved, but the agent claims completion | the same, prefixed "You reported the goal complete, but the goal evaluator says it is not". After 3 such disagreements the run stops (`🎯 goal stopped — the agent reports done but the evaluator disagrees: …`), never as success |
| the remaining list is unchanged (compared case- and punctuation-insensitively, in any order), or `progressed: false`, 3 verdicts in a row | stop as `🎯 goal wedged — no progress across 3 evaluations: <reason>` |
| no verdict (evaluator off, no usable model, call failed or timed out, malformed reply) | exactly the old behaviour: `GOAL_CONTINUE_PROMPT`, task-plan completion, the six-turn stall counter. A failure is reported once per turn as a warning ending "continuing with the standard prompt" |

`GOAL_MAX_ITERS`, `--max-tokens` and `--max-minutes` apply unchanged. A prompt the user queues
while the goal runs still takes the next turn in place of the generated prompt.

The evaluator runs as its own background step between turns, spawned like a turn: the spinner keeps
going, the phone shows the session busy, and Esc or a remote interrupt stops it the same way it
stops a turn.

## 3. Model choice and cost

The evaluator uses the side-call model every other optional side call uses (recap, suggestions,
shell diagnosis): the trivial tier when the session is routed, the first usable non-subscription
trivial model (else the pin) when the session is pinned, never a subscription model the session is
not itself running on, never a CLI bridge, and nothing zero-retention routing forbids. Calls are
capped at 2,048 output tokens at low effort and time out after 90 s.

```toml
[goal]
evaluator = true                          # default
evaluator_model = "groq::llama-3.1-8b-instant"  # optional; same safety rules apply
```

A configured `evaluator_model` that is a foreign subscription, a CLI bridge, or forbidden by
zero-retention routing is ignored for that session in favour of the default side-call model.

Every call is recorded as a `goal/evaluate` side-call usage row, so it is priced into the session
total and shows in per-provider and per-model spend.

## 4. Where `/goal` runs

- Interactive TUI (`forge chat`): `run.rs` turn-done handler.
- Daemon-hosted sessions (`forge serve`, the phone and browser, and `forge run "/goal …"` when it
  is published to a running daemon): the session driver, `run/driver/input.rs`.
- Local headless `forge run "/goal <objective> [--gate "<cmd>"]... [--max-tokens N]
  [--max-minutes N]"` — no daemon running, or `--no-publish-to-fleet`, `--usage-json` or a
  machine `--output-format` (all of which keep the run local): `run/goal_headless.rs`.

The interactive and daemon drivers call `advance_goal` + `launch_goal_step`
(`run/goal_progress.rs`); the headless run has no `App`, so it calls `settle_goal`, the function
`advance_goal` itself ends in. The evaluator, verdict handling, `GOAL_MAX_ITERS`, token/minute
budgets, wedge and disagreement stops, and `--gate` quality gates are therefore the same code.

Headless specifics:

- Each iteration is a turn, then one evaluator call (verdict lines print like in the TUI), then
  the decision. Gates run in the session's workspace root, not the launch directory.
- Exit code: 0 only when the goal is met (evaluator or task plan says so and all gates pass).
  Every other end (budget, iteration ceiling, wedged, gate exhausted, agent/evaluator
  disagreement) prints its reason and exits 1. A turn that ends with open tasks does not fail the
  run the way a plain one-shot does; the next iteration continues it.
- Ctrl-C drops the in-flight turn or evaluation, keeps what streamed, and exits 1
  (`🎯 goal stopped — interrupted`).
- `--usage-json` covers every iteration and evaluator call of the invocation.
- `forge run --tui "/goal …"` is rejected; use `forge chat`.

## 5. Implementation

- `crates/forge-core/src/goal_evaluator.rs` — `GoalVerdict`, `GoalEvalRequest`, the lenient
  `parse_goal_verdict`, the progress line, model selection, and `Session::evaluate_goal`.
- `crates/forge-core/src/goal_evidence.rs` — git and evidence-ledger collection.
- `crates/forge-cli/src/cli/commands/run/goal_progress.rs` — the verdict-aware
  `next_goal_decision`, the re-drive prompt, disagreement and wedge tracking, and the shared
  `advance_goal`/`launch_goal_step` used by both drivers.
- `crates/forge-cli/src/cli/commands/run/goal_headless.rs` — `parse_goal_prompt` and
  `run_goal_headless`, the local one-shot loop (tests: `goal_headless_tests.rs`).
- `crates/forge-provider/src/mock_goal.rs` — the `--mock` provider answers the evaluator from the
  task plan in its input (achieved when every task is done), so offline goals end as before.
- `crates/forge-config/src/goal.rs` — `[goal]`.
