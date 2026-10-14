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

## The script API

- `await tools.<name>({...})` calls any tool the model has, with the same JSON arguments as a
  normal call. It resolves to the tool's output string.
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
subagent tools, `ask_user`, `present_plan`, `update_tasks`, `use_skill`, `remember`, and the
session messaging, dispatch, heartbeat, wakeup and push tools. They drive the session, not the
workspace; call them as normal tools.

## Every inner call is a normal call

Each `tools.<name>()` request is sent back to the session task and runs through
`Session::invoke_tool`, the same broker a normal call uses: task scope, the identical-call guard,
workspace rooting, PreToolUse/PostToolUse hooks, the permission decision with its prompt, the /undo
snapshot, `ToolStart`/`ToolResult` events, the output cap and an audit row under the same
assistant message. After it returns, the same `observe_tool_outcome` bookkeeping that a plain call
gets updates the phantom-edit gate (`mutations_ran`), the verification ledger, the failure-loop
counts and the env-fight cap. A `cargo test` run inside a script counts as verification; a write
inside a script counts as a mutation. The `run_code` wrapper itself is not evidence.

Calls run one at a time, in the order the script makes them. `Promise.all` works but does not run
calls in parallel; writes are therefore always serialized.

Hints an inner call queues (LSP diagnostics after a write, the shell-error diagnosis, hook context,
loop-guard nudges) are appended to that call's value as `[hint] …`, so a program that edits and
then tests sees its own compile errors. They are also delivered once after the outer result.

Only the outer `run_code` result enters the transcript. It ends with a footer:
`[run_code: 7 tool calls, 6 ok, 1 failed, 2.4s]`. When the script fails, the result lists the
calls that already ran (their effects stay, and writes are snapshotted for /undo). Engine errors
use the model's own line numbers.

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

`tools.$search` and name resolution go through one seam, `codemode::catalog::ToolDirectory`.
Today the session implements it over its tool registry plus connected MCP tools. When the shared
deferred-tool registry lands on the direct path, that impl points at the registry and nothing else
in codemode changes.

## Code

| Where | What |
|---|---|
| `forge-core/src/codemode/mod.rs` | spec, argument parsing, the JS wrapper, result rendering |
| `forge-core/src/codemode/runner.rs` | the pump: script future, request channel, deadline, inner dispatch |
| `forge-core/src/codemode/catalog.rs` | `ToolDirectory`, name resolution, deny list, `$search` |
| `forge-core/src/codemode/prompt_clock.rs` | presenter wrapper that pauses the deadline during prompts |
| `forge-core/src/tool_outcome.rs` | `observe_tool_outcome`, shared with the serial tool path |
| `forge-store/src/codemode_store.rs` | `codemode_stats`, `tool_call_rows` |
| `forge-cli/src/cli/commands/stats_codemode.rs` | `forge stats codemode` |
| `forge-config/src/codemode.rs` | `[codemode]` |
