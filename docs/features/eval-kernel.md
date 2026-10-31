# Persistent Python kernel (`python`)

`[harness] eval_kernel = true` (default off) advertises a `python` tool: a Jupyter-style Python kernel, one child process per session, started on first use. Variables, imports and loaded data survive between calls until `reset: true`, a crash, a timeout kill or the end of the session. It is an ordinary `Tool` with `SideEffect::Shell`, so the permission broker, hooks, audit row, full-output spill and task scope already apply. It applies to new sessions.

```toml
[harness]
eval_kernel = true
```

## What the model sees

Arguments: `code` (required), `timeout_secs` (default 60, max 600), `reset`. The cell's `print()` output and a final bare expression (`Out: ...`) come back under a status header:

| Header | Meaning |
| --- | --- |
| `python: ok (cell 4, 0.3s)` | Ran. `; a ! command exited 1` is appended when a `!` command failed; the cell still counts as ok. |
| `python: error KeyError (cell 4)` | Raised; the traceback names `<cell 4>` and its source line. |
| `python: interrupted after 60s; kernel state kept` | Timeout: SIGINT, earlier state kept. |
| `python: kernel killed after ignoring interrupt; state lost` | SIGINT ignored for 3 s, then SIGKILL. |
| `python: kernel died (SIGKILL, likely out of memory); state lost` | The child exited. |
| `python: failed to start (tried ...): ...; use shell` | No usable interpreter. |

A bracketed notice leads the output after any kernel start: `[new kernel: no variables from earlier calls exist]` (also after resume, fork and handoff, where the transcript shows state the new process lacks), `[previous kernel exited (...)]`, `[fresh kernel (reset)]`, `[kernel restarted in <path>]` when the session moved worktrees. Two magics exist: `!cmd` (shell command, output interleaved in order) and `%pip args` (installs into this interpreter). Output is ANSI-stripped, `\r` progress bars collapse to their last state, the model copy is cut to 16,000 characters head plus tail, and `/output` opens the full capture (64 KiB head and tail of the raw output).

## Interpreter discovery

File and PATH lookup only, never a spawn: `<workspace>/.venv/bin/python`, PATH `python3`, PATH `python`, `/opt/conda/bin/python`. Candidates are tried in order until one handshakes (Python 3.8 or newer), within 20 s. A daemon session discovers with the daemon's environment (no `VIRTUAL_ENV`), so a project venv is found only through `<workspace>/.venv`.

## Permission posture

- Default mode asks for every cell; plan mode denies and does not advertise the tool; bypass and accept-edits allow; auto asks. A rule such as `tool = "python"` with `allow = "*"` works.
- "Always" and "this session" answers are downgraded to allow-once and write nothing to config: one click must not approve every later arbitrary cell. Write a config rule for a standing grant.
- Safety floor (every mode, including bypass): `!`/`%pip` lines get the shell privilege and catastrophic checks, and the whole cell is scanned for an inline recursive delete of the home directory. String literals are not scanned, so `open('/etc/passwd')` and `df['su']` are fine. A dynamically built `subprocess.run(['sudo', ...])` hits a per-kernel PATH shim that exits 126 with the hand-off text, so it never reaches PAM.
- Read-only task scopes hide the tool.
- The tool is not registered under `[shell] sandbox = true` (Landlock for the kernel is not implemented), on non-unix platforms, or when no interpreter is found; each case logs a warning.

## Limits

Per-cell wall clock (timeout, SIGINT, SIGKILL); `oom_score_adj = 1000` so the kernel is the OOM victim; head/tail output capture plus the model cap; lifetime is the session (Drop kills the process group, and the kernel kills its own group on EOF of its control pipe, so `process::exit` leaves no orphan); provider keys and `FORGE_DB` are scrubbed from its environment.

## Not covered (compared with `shell`)

No idle reaper (a daemon session holds its kernel until it ends; this gates any default-on flip); `/undo` snapshots and `edits_this_turn`, and `/undo` or checkpoint restore do not reset the kernel; the builtin glob denies and secret-path protection; CC-compat PreToolUse hooks matching Bash or shell; the env-fight nudge, so `%pip` loops escape the spend cap; the shell failure tracker and hook shell-fail; `record_test_proof` and evidence capture; the completeness search detector. Subagents and the MCP bridge keep their own registries and do not see the tool.

## Verification ledger

A kernel cell is a Mutation when it writes files or uses any magic, otherwise Generic, and never a Check. `!pytest` passing is not credited; the guidance sends tests, builds and servers to `shell`. Error, interrupted, died and failed-to-start headers count as failures for the failure-loop guard and the progress and verification ledgers.

## Surfaces

TUI: verb `Python`, headline is the first code line, grouped with shell commands, the approval prompt shows the cell. App: `Python` label, first code line as the headline, the expanded row shows the whole cell, grouped with shell. The curated `harness.eval_kernel` row appears in `/config` and Settings.

## A/B status

Unmeasured. The plan (adoption gate on 11 data-heavy tasks, then the 25-task subset with a paired pass-fraction metric) is in the design notes; the idle reaper must land before any default flip.
