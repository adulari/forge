# Feature: autonomous quality gates + budgets (`/loop`, `/goal`)

> **Status: shipped.** `/loop` and `/goal` accept `--gate "<cmd>"` (repeatable), `--max-tokens N`,
> and `--max-minutes N`. Bare `/loop <task>` and `/goal <objective>` behave exactly as before —
> these are opt-in additions, ported from prime-agent's gate/budget semantics.

## 1. Problem (JTBD)
> When I let `/loop` or `/goal` run unattended, I want it to actually verify its own work (tests
> pass, lint is clean, the build succeeds) before declaring victory — and I want a hard ceiling on
> how much it can spend in tokens or wall-clock time, so an autonomous run can't quietly burn
> through my budget or run forever on a task it can't actually finish.

## 2. Scope

**Gates**
- `--gate "<cmd>"` is repeatable: each is a shell command that must exit 0. Gates only run when
  the model claims the task/goal is genuinely complete (the `LOOP_COMPLETE` sentinel for `/loop`,
  or all tasks done / an explicit `GOAL COMPLETE` for `/goal`) — never on a safety-cap stop
  (iteration ceiling, stall detection).
- Gates run in order; the first failure's bounded output (last ~4000 chars of combined
  stdout+stderr) is fed back to the model as the next iteration's prompt, prefixed
  `` Quality gate `<cmd>` failed: `` — the run keeps going instead of stopping.
- Each gate gets up to 3 attempts (`DEFAULT_GATE_RETRIES`) before the whole run stops with
  `◆ loop stopped — quality gate exhausted: <cmd>` (or the `🎯 goal` equivalent) — never reported
  as success. Each attempt is capped at 300s (`DEFAULT_GATE_TIMEOUT`); a gate that outruns its
  timeout has its whole process tree killed (SIGTERM→grace→SIGKILL on Unix, `taskkill /F /T` on
  Windows).
- If the workspace hasn't changed since a gate's last failure (a hash of `git status --porcelain`
  + `git diff HEAD`), the gate is NOT re-run on the next attempt — a certain repeat would just
  burn the retry budget — but it still counts as another attempt and replays its prior bounded
  output. Outside a git repo the fingerprint is always empty, so gates always rerun there.

**Budgets**
- `--max-tokens N` and `--max-minutes N` cap accumulated input+output tokens and wall-clock
  elapsed for the whole `/loop`/`/goal` run. Checked between iterations (including between gate
  retries); reaching either stops the run with `◆`/`🎯 ... stopped — token/time budget exhausted`
  — never success.
- Unset (the default) means unbounded, identical to today's behavior.

**Non-goals**
- No per-gate retry/timeout override (one `GateConfig` applies to every gate in a run).
- No change to `/loop`/`/goal`'s existing sentinel, iteration-cap, or stall-detection policy —
  gates and budgets are additive checks layered on top.

## 3. Usage

```
/loop [--gate "<cmd>"]... [--max-tokens N] [--max-minutes N] <task>
/goal [--gate "<cmd>"]... [--max-tokens N] [--max-minutes N] <objective>
```

```
/loop --gate "cargo test" --gate "cargo clippy --all-targets" --max-tokens 200000 ship the parser
```

Options may appear in any order, before or interleaved with the prompt text; a gate command
containing spaces must be quoted (`--gate "cargo test"`) — parsed quote-aware via `shell_words`.
The bare form (no flags) is untouched byte-for-byte for backward compatibility.

## 4. Implementation

- `crates/forge-cli/src/cli/commands/run/gates.rs` — the gate engine: `GateSpec`/`GateState`/
  `GateConfig`, `run_gates` (async — spawns real shell commands), `workspace_fingerprint` (async
  — shells out to `git`), and the pure `bound_output` truncation helper.
- `crates/forge-cli/src/cli/commands/run/autonomous.rs` — `AutonomyBudget`, the pure
  `loop_budget_stop_reason`/`goal_budget_stop_reason` checks, and `next_loop_decision`/
  `next_goal_decision`: the async orchestration that folds gates + budgets into the existing
  `loop_stop_reason`/`goal_stop_reason` policy, mutating `LoopState`/`GoalState` in place so gate
  attempts/fingerprints and accumulated token usage persist across iterations of one run.
- `crates/forge-tui/src/commands.rs` — `parse_autonomy_options` parses the shared flags out of the
  raw `/loop`/`/goal` arg string.
- Wired at both turn-completion sites that drive `/loop`/`/goal`: the interactive TUI loop
  (`run.rs`) and the headless `forge serve` driver (`run/driver/input.rs`) — both call the same
  `next_loop_decision`/`next_goal_decision` so the behavior is identical on a phone-driven session
  and a local terminal.
