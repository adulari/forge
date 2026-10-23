# Shell error interceptor — fail → explain + fix, no prompt

> Status: **MVP done** — a failed `shell` command is auto-diagnosed by a pattern table or one cheap
> model call that runs off the turn. On by default; `[shell] explain_errors = false` disables it.

## Why

The Helm-note vision lists "command fails → AI auto-explains + offers a fix, no prompt needed"
as a Wave-4 differentiator. Bash is the owner's #1 tool (5,909 uses); failed commands are a
constant, and today every explanation costs a manual round-trip. The interceptor removes that
round-trip: the moment a shell command fails, Forge attaches a terse likely-cause + concrete
fix.

## What shipped

- After the `shell` tool returns a **failure** — non-zero exit, signal, timeout, or spawn
  error — Forge explains the likely cause and a concrete fix in one or two lines and emits
  `PresenterEvent::ShellDiagnosis` (rendered as `⚠ shell failed <cmd>` + the dimmed explanation
  in both the TUI and headless output).
- **Common failures are answered from a table** (`pattern_diagnose`): command not found, a
  missing path, permission denied, a port in use, a refused connection, a full disk, out of
  memory. No model call; the answer is ready with the tool result.
- **Anything else is one trivial-tier model call that never holds the turn.** It runs on a
  detached task and the failed command's result goes back to the model at once
  (`crates/forge-core/src/shell_diagnosis.rs`). The diagnosis reaches the model only when it is
  ready by the next request: arriving later it would describe a command the model has already
  acted on. The person sees it whenever it lands: at the next step while the turn runs, at once on
  the TUI after the turn is over. Inline, it held the tool result for 2.7-3.1 s in 29 of 52
  `inferhub::cb/deepseek-v4.1-flash` bench tasks.
- **No call where the output already is the answer:** a verification run that reported its
  verdict (tests, build, lint, type check, as the completion gate classifies them), a failed
  assertion (`AssertionError`, `assertion failed`), and a search or comparison whose exit 1 means
  "no match" or "differs" (`grep`, `rg`, `git grep`, `diff`, `cmp`, `test`). These were most of
  the calls the bench made: restating a test's assertion message, or "exit 1 is git grep's no
  matches status".
- **No call on an unattended surface whose side calls run on the session's own pinned model**
  (`forge run --model X` in print, JSON or stream-json mode, `forge bench`): the only reader would
  be that same model, which reads the failure itself on its next step. A TUI keeps the call (in any
  permission mode), and so does `mesh.pin_failover = true`.
- **Best-effort:** skipped when the budget is exhausted, bounded at 12 s, silent on a model error
  (the model is benched as for any failed side call).
- Gated by `[shell] explain_errors` (default `true`).
- **A pinned session diagnoses on its pin.** With `--model` or `/model`, the call goes to the
  pinned model (or is skipped when the pin is a subscription) instead of the cheapest trivial-tier
  route, so a pinned run never reaches another provider (see mesh-routing.md, "Side calls stay on
  the pin"; `mesh.pin_failover = true` restores the trivial-tier route).

## Design

- **Failure detection:** `shell_command_did_fail(result)` in `forge-core` is pure (unit-tested) —
  it reads the tool's first line (`shell: exit N …` / `timed out` / `error:` / `failed to
  start`); only `exit 0` is success.
- **Skip rules:** `diagnosis_adds_nothing(command, result)` (pure, unit-tested) and
  `Session::diagnosis_reaches_no_one`.
- **The call:** `SideModelInputs::decide` (the side-call model rules recap and memory use) and one
  `provider.complete_with` with a fixed system prompt (`SHELL_DIAGNOSE_SYSTEM`) and the command +
  result as the user message, on a `tokio::spawn`ed task. Its result comes back on a channel the
  session drains at each step boundary (`Session::deliver_shell_diagnoses`, never waiting) and once
  the turn ends (`Session::close_shell_diagnoses`, which hands a channel-backed surface what is
  left; later results go to that surface directly).
- **Wiring:** in `invoke_tool`, after the PostToolUse hooks and before returning, when
  `side_effect == Shell && config.shell.explain_errors && shell_command_did_fail(&result)`; the
  concurrent read-only batch does the same for each failed shell call.

## Shipped (follow-up)

- **Transcript injection** — a diagnosis is injected as a hint note: a table answer right after
  the tool result, a model answer at the next step boundary if it is ready by then, so the model
  sees the likely cause and fix on its next completion.
- **Usage recording** — `Store::record_side_call_usage` inserts a synthetic inactive message row
  as the FK anchor and records the diagnosis call's tokens/cost in the `usage` table, so daily
  and monthly budget queries stay accurate. `compact()`'s summarise call is also recorded the
  same way.

## Fix command

- Diagnosis prompt outputs structured `cause` + optional `FIX: <cmd>` line, shown in the diagnosis
  card and sent to the model with the failed result.
- There is no key to copy the fix into the input. An earlier `F` shortcut did that, but it fired on
  any `f` typed while idle with a fix pending, replacing the prompt being written (removed).

