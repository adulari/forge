# Bench task suites, trajectory export and `run --usage-json`

Three pieces that make Forge measurable outside SWE-bench and let the results be shared:

- `forge bench tasks --suite <dir>` runs a Terminal-Bench-style suite in fresh temp workspaces.
- `forge bench export` writes one redacted trajectory JSONL per attempt.
- `forge run --usage-json <path>` reports what a single headless run consumed, for any harness
  that drives Forge as a subprocess.

A fourth command, `forge bench recall`, measures context recall after compaction and resume; it
is in [bench-recall.md](bench-recall.md).

The scoring side (`bench report`, pass@1, cost per solved task) is in
[bench-metrics.md](bench-metrics.md); integrity checks are in
[bench-integrity.md](bench-integrity.md).

## Task suites

A suite is a directory with one subdirectory per task:

```
my-suite/
  fix-config/
    instruction.md   # the prompt, sent verbatim
    setup.sh         # optional; prepares the workspace
    test.sh          # required; exit 0 = solved
```

Each attempt of each task:

1. creates a fresh workspace (`forge-task-<name>-XXXX`, removed afterwards) under `--workdir`,
   else under `TMPDIR` (the system temp dir; often a RAM disk, so point `--workdir` at a disk
   path for tasks that build or install);
2. runs `bash setup.sh` in it; a non-zero exit or a timeout skips the agent and the test;
3. commits the post-setup state as a git baseline (unless setup made its own repo), so the
   agent's changes can be read back as a diff (`patched` in the metrics);
4. runs the agent (Bypass mode, no prompts) on `instruction.md` with the workspace as its cwd;
5. stages the workspace and records the agent's patch (see "The patch and `git add -A`" below);
6. runs `bash test.sh` in the workspace and records its exit code and duration (a `test.sh` that
   cannot start is recorded as the attempt's error; the agent's session and usage are kept either
   way);
7. kills every process group the scripts started.

`setup.sh` and `test.sh` are run from the task directory, never copied into the workspace, so the
agent is not handed the grader. Both get `TASK_DIR` (the task directory) and `WORKSPACE` in their
environment. Each script runs in its own session (`setsid`), so a server `setup.sh` starts in the
background keeps running while the agent works and is killed when the attempt ends. Script
output goes to `<out-stem>.logs/<task>.attempt<k>.{setup,test}.log`.

```
forge bench tasks --suite my-suite --model bedrock::global.moonshotai.kimi-k3 --attempts 3
forge bench report --run tasks.seed1.metrics.jsonl=tasks.seed1.eval.json \
                   --run tasks.seed2.metrics.jsonl=tasks.seed2.eval.json \
                   --run tasks.seed3.metrics.jsonl=tasks.seed3.eval.json
```

| flag | default | meaning |
|---|---|---|
| `--suite` | required | the suite directory |
| `--out` | `tasks.jsonl` | per-attempt results; the sidecars are named after it |
| `--tasks` | all | task names or globs (`*`, `?`), comma-separated or repeated |
| `--skip` | none | task names or globs to leave out; applied after `--tasks` |
| `--agent` | `forge` | `forge`, `claude-code` or `codex` (same harnesses as `bench swe`) |
| `--clean` | off | `--agent claude-code` only: run it with `--setting-sources project --strict-mcp-config` |
| `--model` | mesh | pin a model |
| `--attempts` | 1 | >1 writes one file set per seed (`tasks.seedK.*`) |
| `--timeout-secs` | 1200 | budget for setup.sh, the agent turn and test.sh, each |
| `--limit` | all | first N of the selected tasks, in name order |
| `--workdir` | `TMPDIR` | directory the attempt workspaces are created in |
| `--mock` | off | offline mock provider, for smoke-testing a suite (Forge agent only) |

Selected tasks always run in suite (name) order, whatever order `--tasks` lists them in. A
`--tasks` or `--skip` pattern that matches no task is an error that lists the available names, so
a misspelt name cannot silently run less than you asked for; so is a selection that leaves nothing.
`--limit` is applied after both. The run prints where the workspaces go before it starts.

Outputs, per seed:

- `<out>`: one row per task with `solved`, `setup_exit`, `test_exit`, `test_timed_out`,
  `session_id`, plus `agent_secs` (the agent turn), `test_secs` (`test.sh`) and, for an unsolved
  attempt, `reason`. These three are omitted when they do not apply (the agent never ran, or
  `test.sh` did not start).
- `<out-stem>.metrics.jsonl`: the same `InstanceMetric` rows `bench swe` writes (tokens, cache
  splits, cost, notional cost, calls, outcome, session id), so `bench report` and `bench export`
  read it unchanged. Fields that only some agents have are listed under "Claude Code runs".
- `<out-stem>.eval.json`: `resolved_ids` / `unresolved_ids`, the shape `bench report --run` pairs
  with a metrics file.

### Reading a failed attempt

Each task prints a verdict line as soon as it is graded, and the run ends with a table of every
attempt:

```
  ✗ test.sh exited 1 in 0.5s · agent 42.3s: FAIL: total is 2, want 3
...
task          result     agent     test  note
fix-config    PASS       42.3s     0.5s
limiter       FAIL      118.0s     0.4s  FAIL: total is 2, want 3
```

`reason` (also the table's `note`) is one line of the failed `test.sh`'s output, at most 160
characters with colour codes removed: the last line containing `FAIL`, else the last line that
looks like an error (`error`, `panicked at`, `Traceback`, `assert`), else its last line. A timeout
says so, and a failure before grading (a failed `setup.sh`, no workspace) says what stopped the
attempt. When the agent turn itself failed, `agent error: …` leads the reason, because the test
output is then usually only a symptom. Keep `test.sh` output short and put its verdict last; the
full output is in `<out-stem>.logs/<task>.attempt<k>.test.log`.

### The patch and `git add -A`

After the agent's turn Forge runs `git add -A` in the workspace and reads the patch with
`git diff --cached <baseline>`. This is the only way to see files the agent *created*, which a
plain `git diff` ignores, and diffing against the baseline commit rather than `HEAD` keeps the
patch whole when the agent commits its own work. Two consequences for `test.sh`, which runs
afterwards in the same workspace:

- everything the agent changed is already staged, so a bare `git diff` prints nothing; use
  `git diff --cached` (or `git diff HEAD`) to see the agent's changes;
- `.forge/` and Python bytecode are left out of the patch and unstaged.

The staging happens for every agent, including `claude-code` and `codex`.

### Outcome

The `outcome` column of the metrics rows says how an attempt ended: `ok` (the workspace
changed and nothing went wrong), `no_change` (the turn ended with the workspace exactly as it
started and no refusal, timeout or tool outage explains why), `refused`, `timed_out`,
`tools_unavailable`. `no_change` is still scored by `test.sh`; it exists so a run that did nothing,
such as `--mock` or an agent that only talked, does not read as an ordinary `ok`. `outcome_detail`
repeats the turn's error when there was one.

### Claude Code runs

`--agent claude-code` runs `claude -p --output-format stream-json --verbose
--dangerously-skip-permissions` and reads the stream, not just the final object. The metrics rows
gain what Claude Code itself reports:

| field | source |
|---|---|
| `model_calls` | the result's `num_turns` |
| `tool_calls`, `tool_errors` | `tool_use` blocks, and `tool_result` blocks flagged `is_error`, of the main conversation (sub-agent traffic, which carries `parent_tool_use_id`, is not counted) |
| `duration_ms`, `ttft_ms` | the result's `duration_ms` and `ttft_ms`: Claude Code's own timing of the turn and its time to first token |
| `agent_secs` | Forge's wall clock around the same turn (includes process start-up) |
| cost, tokens | the result's `total_cost_usd` and `usage`, as before |

A run that is killed at the timeout never prints its result object. It keeps its tool and turn
counts (counted from the messages it did print) but no tokens, cost or timings, and
`metrics_complete` stays false. The older single-object `--output-format json` output still parses,
without tool counts.

By default Claude Code loads your user settings, hooks, `CLAUDE.md`, skills and MCP servers, which
costs tokens and time and varies from machine to machine. `--clean` adds `--setting-sources project
--strict-mcp-config`, so only what the workspace itself holds configures it. Use it for
comparisons; leave it off to measure Claude Code as you run it.

### `--mock`

`--mock` is a wiring test of the suite, not a measurement. It pins the offline mock provider, so it
neither routes through the mesh nor spends a call classifying the task, and it never prints the
CLI-bridge terms notice that routing to `claude-cli` would. The mock turn changes nothing, so
every attempt has outcome `no_change` and a suite scores only the tasks whose `test.sh` passes on
the untouched workspace. The run says this on stderr.

What this is not: there is no container per task. The workspace is a temp directory on the host
and the scripts run as your user, so only run suites you trust. Terminal-Bench tasks that assume
an absolute path such as `/app` or a Docker image need adapting (point them at `$WORKSPACE`).
The deterministic integrity checks of `bench swe` are not run on suite attempts; they are
written around SWE-bench's FAIL_TO_PASS and test-file conventions.

An example suite lives in `crates/forge-cli/tests/fixtures/bench-suite/`; with `--mock` it
scores 1/2 (`tests/bench_suite_e2e.rs`).

## Trajectory export

```
forge bench export --metrics tasks.metrics.jsonl --out trajectories/
```

For each metrics row, `bench export` loads the attempt's Forge session from the store (the same
`load_replay` read path as `forge replay`) and writes `<out>/<task>.attempt<k>.jsonl`:

- line 1, `"type": "attempt"`: task, agent, model, attempt, session id, outcome and
  `outcome_detail`, `patched`, `timed_out`, the integrity verdict and findings (from the
  `.integrity.jsonl` next to the metrics file, or `--integrity <file>`), usage totals, step
  count and `trajectory` (`forge_session`, `missing_from_store`, or
  `unavailable_no_forge_session` for external CLI agents, which get a header-only file);
- every further line, `"type": "step"`: one transcript message in order with its role, model,
  content, tool calls (name and arguments) and, for assistant messages, its usage row.

A bridged `claude-cli` / `codex-cli` session runs its tools inside the CLI, so its assistant
messages hold no tool calls of their own. The tools the CLI reported are written to the audit
ledger as the turn is recorded (permission `bridge`), and `load_replay` fills them into the
assistant entry, so the exported steps and `forge replay` show what the agent did. Where a tool
call's result is known its status is `ok` or `error`; a call that never reported back is
`unfinished`. A message that has tool calls of its own keeps those.

Every string is passed through the shared [redactor](redaction.md) and then capped at 8 KB
(`…[truncated N bytes]`). Redaction runs first, so truncation never leaves half a secret behind.
Run export with `FORGE_DB` pointing at the store the benchmark used. Regex redaction is not
complete; review a trajectory before publishing it.

## `forge run --usage-json <path>`

After the turn (also after a failed or interrupted one), writes one JSON object:

```json
{
  "session_id": "…",
  "model_calls": 5, "tool_calls": 1,
  "input_tokens": 174, "cached_input_tokens": null, "cache_write_tokens": null,
  "reasoning_tokens": null, "output_tokens": 72,
  "cost_usd": 0.0, "notional_cost_usd": null,
  "wall_secs": 0.11
}
```

`null` means no call reported that split, never zero. The numbers come from the session's usage
rows (`SessionUsageDetail`, the same source as the bench metrics), counted from just before the
turn, so a `--resume`d session reports this invocation only, not its earlier turns. `--usage-json` keeps the run in
this process (it implies `--no-publish-to-fleet`), because a run handed to the daemon would leave
nothing to report.

## Code

- `crates/forge-cli/src/bench/suite.rs` — suite discovery, script runner, result files.
- `crates/forge-cli/src/bench/suite_select.rs` — `--tasks` / `--skip` matching.
- `crates/forge-cli/src/bench/suite_report.rs` — failure reason and the per-attempt table.
- `crates/forge-cli/src/bench/external_agent.rs` — Claude Code / Codex subprocess and its output
  parsing.
- `crates/forge-core/src/bridge_trace.rs` — bridged-CLI tool calls and model calls into the ledger.
- `crates/forge-cli/src/bench/export.rs` — trajectory export.
- `crates/forge-cli/src/bench.rs` — `run_agent`, the agent turn shared by `bench swe` and
  `bench tasks`.
- `crates/forge-cli/src/cli/commands/run/usage_json.rs` — `--usage-json`.
