# Codemode (`run_code`)

`run_code` lets a direct-path model write a short JavaScript program that calls Forge's other tools
and returns only what it needs. One model step can then search, read the matches, edit, and run the
tests, and only the lines the program keeps reach the context window.

```js
const hits = await tools.search({ query: "fn parse_config", path: "crates" });
const file = hits.split("\n")[0].split(":")[0];
await tools.edit_file({ path: file, old: "pub fn parse_config", new: "pub(crate) fn parse_config" });
const out = await tools.shell({ command: "cargo check -p forge-agent-config" });
return out.split("\n").slice(0, 20).join("\n");
```

## When the model reaches for it

Advertising the tool was not enough: before the guidance below, `inferhub::glm-5.3-flash` made zero
`run_code` calls in 108 benchmark runs and spent one model step per read, search or shell probe.
Three things steer it now, all direct-path only:

- **The tool description** leads with the job ("batch several tool calls into ONE step"), the rule
  (use it when you can already name the next 2+ calls) and the exact call shape with examples.
- **A guidance block** (`codemode::guidance::GUIDANCE`) is sent as its own system message right
  after the base prompt whenever `run_code` is in the request's tool list. It splits the work:
  independent reads and searches go out as parallel calls (already one step), while a dependent
  chain the model can already plan (edit then test, search then read the hits, run the tests and
  keep only the failures) goes into one `run_code` call. It states the
  `await tools.<tool_name>({...})` shape, that every result is one string (glob and search too),
  to return only what is needed, and a three-line edit-then-test example. It is constant text
  placed before the transcript, so the cached prefix stays byte-stable; CLI bridges never get it
  because their tool list never contains `run_code`.
- **A one-time hint**: after three consecutive steps that each made a single exploration call
  (`read_file`, `search`, `glob`, `list_dir`, `shell`, ...), the loop appends one system hint after
  that step's tool results: plan the next few calls, send independent ones in parallel and a
  dependent chain through `run_code`. At most once per turn;
  a multi-call step, an edit or a `run_code` call resets the streak. It is an advisory nudge, so
  the `minimal` harness profile turns it off, and it never fires on a CLI bridge, with codemode
  disabled, or when the task scope excludes `run_code`.

## The script API

- `await tools.<name>({...})` calls any tool the model has, with the same JSON arguments as a
  normal call. It resolves to the tool's output string. A bare string is accepted as shorthand:
  `tools.read_file("a.py")` becomes `{path: "a.py"}` (the tool's first required parameter, else
  `path`/`command`/`query`/`pattern`), and a string holding a JSON object is parsed.
- Every result is a string, including glob and search output (newline-separated). Models
  habitually treat that output as an array, so inside the sandbox `String.prototype.join` exists
  and returns the string unchanged: `files.join("\n")` works instead of throwing.
- It throws an `Error` when the call did not run: denied, blocked by a hook, refused by the task
  scope or a loop guard, unknown tool, or invalid arguments. A shell command that ran and exited
  non-zero resolves, so a script can read failing test output without a `try`/`catch`; check the
  `shell: exit N` first line.
- `await tools.$search("keywords")` returns up to eight `{name, description, schema}` objects for
  matching tools, deferred ones included, so finding a tool does not cost a model round trip.
  `tools.tool_call({name, arguments})` and the `mcp__forge__` / `default_api:` spellings resolve to
  the real tool.
- `console.log(...)` (and `log`) lines are kept (up to 200) and shown under `[logs]`, even when the
  script throws.
- The code runs inside an async function: end it with `return <value>`. A string comes back as-is,
  anything else as pretty JSON.
- The argument is `code` (a ```` ```js ```` fenced block is accepted, for models that mangle
  escaped JSON) and an optional `timeout_secs`.

There is no `fs`, `fetch`, `require` or timer API. The sandbox is the forge-workflow QuickJS
engine, whose only capability here is the `tools` bridge.

Not callable from a script: `run_code` itself, `run_workflow`, `spawn_agents` and the other
subagent tools, `ask_user`, `present_plan`, `use_skill`, `remember`, and the session messaging,
dispatch, heartbeat, wakeup and push tools. They drive the session, not the workspace; call them
as normal tools. `update_tasks` is the exception: it only rewrites the task checklist, so a script
can tick a task off and run the next calls in the same step. (It used to be denied, which killed a
script that led with a task update on its first line and threw away every call batched after it.)

## Every inner call is a normal call

Each `tools.<name>()` request is sent back to the session task and runs through
`Session::invoke_tool`, the same broker a normal call uses: task scope, the identical-call guard,
workspace rooting, PreToolUse/PostToolUse hooks, the permission decision with its prompt, the /undo
snapshot, `ToolStart`/`ToolResult` events, the output cap and an audit row under the same
assistant message. After it returns, the same `observe_tool_outcome` bookkeeping that a plain call
gets updates the phantom-edit gate (`mutations_ran`), the verification ledger, the progress ledger's
observations and the env-fight cap. A `cargo test` run inside a script counts as verification; a write
inside a script counts as a mutation. The `run_code` wrapper itself is not evidence: it counts
as a failure only when the script threw, failed or ran out of time (not when it returns a failing
test's output), so a script that re-runs the same failing test every step is still a stale step
for the progress ledger and ends in a nudge and a stop. The footer's timing is ignored when
judging whether a step brought new information.

Calls run one at a time, in the order the script makes them. `Promise.all` works but does not run
calls in parallel; writes are therefore always serialized.

Hints an inner call queues (LSP diagnostics after a write, the shell-error diagnosis, hook context,
loop-guard nudges) are appended to that call's value as `[hint] …`, so a program that edits and
then tests sees its own compile errors. A hint the script did not return is delivered once after
the outer result; one it returned is not repeated. LSP blocks are per-file deltas capped at about
2,000 characters (`lsp-diagnostics.md`).

Only the outer `run_code` result enters the transcript. It ends with a footer:
`[run_code: 7 tool calls, 6 ok, 1 failed, 2.4s]`. When the script fails, the result lists the
calls that already ran (their effects stay, and writes are snapshotted for /undo). A rejected
inner call names itself: `tools.read_file({"path":"x.py"}) failed: io error: ...`. Engine errors
use the model's own line numbers, and a thrown error (including a rejected tool call) is prefixed
with the line of the model's code it surfaced on (`error: line 3: TypeError: ...`), read from the
error's stack.

## Measured adoption

Six small Python tasks (fix a bug, add a CLI flag, rename a method, answer a question, add a
feature with tests), `forge run --mode bypass`, v2.19.0 against this change, three reps per task
per model, interleaved, all on InferHub. Means per run:

| model | build | pass | runs using run_code | run_code steps / tool steps | input tokens | output tokens | model calls | wall s |
|---|---|---|---|---|---|---|---|---|
| glm-5.3-flash | v2.19.0 | 18/18 | 0/18 | 0/113 | 112,645 | 979 | 8.9 | 62.0 |
| glm-5.3-flash | guided | 18/18 | 4/18 | 5/110 | 108,791 | 1,229 | 8.5 | 69.0 |
| kimi-k3 | v2.19.0 | 18/18 | 0/18 | 0/113 | 108,940 | 2,222 | 9.1 | 124.0 |
| kimi-k3 | guided | 18/18 | 6/18 | 8/109 | 105,208 | 2,019 | 8.5 | 109.1 |

What the numbers say: both models now use `run_code`, mostly as intended (edit then run the
tests, final diff plus a grep sweep), with zero script errors in the final round. On tasks this
short the effect on cost is small and inside run-to-run noise: per-task input varies by up to
±50% between identical runs, driven mostly by how many edits trigger an LSP diagnostics block.
Both models already send independent reads as parallel calls, which is why the guidance steers
`run_code` toward dependent chains instead of re-wrapping parallel batches (an earlier wording
that said "2+ calls" got more adoption, 6/18 glm runs, but mostly serialized calls the model would
have sent in parallel anyway, and its first version hit `TypeError: not a function` on
`glob(...).join` in 5 of 15 scripts).

## Limits

- Deadline: `timeout_secs` (default 300, at most `max_timeout_secs`). It is checked only between
  inner calls: a call in flight is never cancelled, so its snapshot, write and audit row always
  complete. Time spent waiting on a permission prompt does not count. After the deadline, further
  calls throw `run_code deadline exceeded`; the script gets two seconds to return partial results
  and is then stopped.
- `max_calls` (default 40) inner calls per script; later calls throw.
- 64 MB heap and a 5 s cap on any stretch of pure JS without a tool call.
- `result_max_chars` (default 16,000) characters of output, footer kept.
- Calls a script starts but never awaits may not run. Always `await`.

## Configuration

```toml
[codemode]
enabled = true          # kill switch: false hides the tool and refuses calls to it
timeout_secs = 300
max_timeout_secs = 900
max_calls = 40
result_max_chars = 16000
```

Codemode is on by default for direct-path models. CLI bridges (claude-cli, codex-cli) never see
it: they serve Forge's tools through `forge mcp-serve`, which has its own registry and batches in
the CLI's own loop.

## Metrics

`forge stats codemode [--session [ID]] [--days N]` reads the audit trail and reports:

- `run_code` calls and the **script error rate** (thrown error, parse error or deadline; inner-call
  failures the script handled do not count);
- inner tool calls per run;
- **output tokens per step** that emitted `run_code`, next to the same figure for every other tool
  step. A program is model output, priced well above input, so this is the cost side of the trade.

## Deferred tools

`run_code` is part of the direct path's advertised core (`deferred_tools::DIRECT_CORE_EXTRA`), so
the lean surface never hides it behind `tool_search`. The tools the lean surface does defer
(`notebook_edit`, `web_fetch`, `open_pr`, ...) stay in the session's registry, so a script calls
them by bare name or as `tools.tool_call({name, arguments})` without a `tool_search` round trip.

`tools.$search` and name resolution go through `codemode::catalog::ToolDirectory` and reuse the
shared deferred registry ([deferred-tools.md](deferred-tools.md)): the same `tool_call` unwrap
(object, escaped-string, `args`/`input` arguments), the same exact-name-then-keyword ranking, and
the same routing of an external `server__tool` name through `mcp_call`. The difference is scope:
`$search` covers every tool a script may call, core included, while `tool_search` lists only the
deferred ones. An inner call to a deferred tool meets the same gate as a plain call: in Plan mode
a deferred write is denied and throws in the script, and only a write that succeeded counts toward
the turn's mutations.

## Code

| Where | What |
|---|---|
| `forge-core/src/codemode/mod.rs` | spec, argument parsing, the JS wrapper, result rendering |
| `forge-core/src/codemode/runner.rs` | the pump: script future, request channel, deadline, inner dispatch |
| `forge-core/src/codemode/catalog.rs` | `ToolDirectory`, name resolution, deny list, `$search` |
| `forge-core/src/codemode/guidance.rs` | the guidance block, the exploration-streak hint |
| `forge-core/src/codemode/prompt_clock.rs` | presenter wrapper that pauses the deadline during prompts |
| `forge-core/src/tool_outcome.rs` | `observe_tool_outcome`, shared with the serial tool path |
| `forge-store/src/codemode_store.rs` | `codemode_stats`, `tool_call_rows` |
| `forge-cli/src/cli/commands/stats_codemode.rs` | `forge stats codemode` |
| `forge-config/src/codemode.rs` | `[codemode]` |
