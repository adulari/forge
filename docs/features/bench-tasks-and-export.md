# Bench task suites, trajectory export and `run --usage-json`

Three pieces that make Forge measurable outside SWE-bench and let the results be shared:

- `forge bench tasks --suite <dir>` runs a Terminal-Bench-style suite in fresh temp workspaces.
- `forge bench export` writes one redacted trajectory JSONL per attempt.
- `forge run --usage-json <path>` reports what a single headless run consumed, for any harness
  that drives Forge as a subprocess.

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

1. creates a fresh temp workspace (`forge-task-<name>-XXXX`, removed afterwards);
2. runs `bash setup.sh` in it; a non-zero exit or a timeout skips the agent and the test;
3. commits the post-setup state as a git baseline (unless setup made its own repo), so the
   agent's changes can be read back as a diff (`patched` in the metrics);
4. runs the agent (Bypass mode, no prompts) on `instruction.md` with the workspace as its cwd;
5. runs `bash test.sh` in the workspace and records its exit code;
6. kills every process group the scripts started.

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
| `--agent` | `forge` | `forge`, `claude-code` or `codex` (same harnesses as `bench swe`) |
| `--model` | mesh | pin a model |
| `--attempts` | 1 | >1 writes one file set per seed (`tasks.seedK.*`) |
| `--timeout-secs` | 1200 | budget for setup.sh, the agent turn and test.sh, each |
| `--limit` | all | first N tasks in name order |
| `--mock` | off | offline mock provider, for smoke-testing a suite (Forge agent only) |

Outputs, per seed:

- `<out>`: one row per task with `solved`, `setup_exit`, `test_exit`, `test_timed_out`,
  `session_id`.
- `<out-stem>.metrics.jsonl`: the same `InstanceMetric` rows `bench swe` writes (tokens, cache
  splits, cost, notional cost, calls, outcome, session id), so `bench report` and `bench export`
  read it unchanged.
- `<out-stem>.eval.json`: `resolved_ids` / `unresolved_ids`, the shape `bench report --run` pairs
  with a metrics file.

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
rows (`SessionUsageDetail`, the same source as the bench metrics). `--usage-json` keeps the run in
this process (it implies `--no-publish-to-fleet`), because a run handed to the daemon would leave
nothing to report.

## Code

- `crates/forge-cli/src/bench/suite.rs` — suite discovery, script runner, result files.
- `crates/forge-cli/src/bench/export.rs` — trajectory export.
- `crates/forge-cli/src/bench.rs` — `run_agent`, the agent turn shared by `bench swe` and
  `bench tasks`.
- `crates/forge-cli/src/cli/commands/run/usage_json.rs` — `--usage-json`.
