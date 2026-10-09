# Feature: pre/post tool-use shell hooks

> **Status: extended.** `[[hooks]]` config entries run a shell command around tool calls and
> session lifecycle events. `PreToolUse` blocks a call; `PostToolUse` observes; `UserPromptSubmit`
> can rewrite or block a user prompt; `SessionStart`/`SessionEnd` fire at session boundaries.
> Wired into both the direct tool path and the plain/TUI chat loops.

## 1. Problem (JTBD)

> When I run Forge, I want my own shell commands to run automatically around tool calls — to
> enforce a policy, rewrite/observe a command, or trigger side work (re-index, notify) — so my
> environment behaves like my heavily-hooked Claude Code setup instead of a bare agent.

The owner's whole environment is hook-driven (a token-proxy on every command, a graph injector
after edits, auto-title). Without hooks, none of that carries over. This MVP gives the two
load-bearing events.

## 2. Scope (MoSCoW)

**Must have (shipped)**
- `[[hooks]]` config: `event` (`pre_tool_use` | `post_tool_use`), optional `matcher`
  (tool-name filter), `command` (POSIX `sh -c`), `timeout_secs` (default 30).
- The tool call is passed to the hook as JSON on **stdin** (`{tool, args}` for pre,
  `{tool, args, result, ok}` for post).
- **PreToolUse blocks on non-zero exit**: the tool does not run; the hook's stderr (or stdout)
  becomes the result the model sees (`blocked by hook: <reason>`).
- **PostToolUse observes**: stdout is surfaced as a note; exit code is advisory.
- Time-bounded: a hook that exceeds `timeout_secs` is killed (`kill_on_drop`) and noted, never
  hangs the turn. Inert (zero overhead) when no hooks are configured.

**Shipped (follow-up)**
- `UserPromptSubmit` — fires before each agent turn; hook stdout replaces the prompt on exit 0;
  non-zero blocks the turn with stderr as the reason. Enables RTK-style prompt rewriting.
- `SessionStart` / `SessionEnd` — observe-only lifecycle events; fire in both TUI and plain chat
  loops. Payload: `{"session_id": "<id>", "event": "session_start|session_end"}`.

**Shipped (arg rewriting)**
- `PreToolUse` exit 0 + JSON object on stdout → rewrites tool args before the tool runs.
  Exit 0 + plain text → note only (unchanged args). Exit non-zero → block.

**Shipped (structured directive protocol)**
- A hook can emit a JSON object with an explicit `"action"` field on stdout (exit 0) to do more
  than rewrite — the same protocol works for `PreToolUse` **and** `PostToolUse`:
  - `{"action":"rewrite","args":{…}}` — replace the tool's args (PreToolUse).
  - `{"action":"inject","context":"…"}` — inject model-visible context: queued as a system hint and
    shown to the model right after the tool result (e.g. lint output, "this file is generated", a
    policy reminder). No block, no rewrite. The first capability that lets a hook *teach* the model,
    not just gate it.
  - `{"action":"block","reason":"…"}` — block the call (PreToolUse). On `PostToolUse` (the call has
    already run, nothing to unwind) it degrades to a note.
  - `{"action":"allow"}` — explicit no-op (approve without changing anything).
  - An unrecognised `action`, or a malformed directive (missing `args`/`context`), degrades to a note
    so the author sees their output instead of it vanishing.
- **Back-compatible:** a bare JSON object with no `"action"` field keeps the legacy meaning
  (rewrite args, PreToolUse only); plain text is still a note; exit non-zero is still a hard block.

**Shipped (MCP tool hooks)**
- `PreToolUse` and `PostToolUse` now fire for MCP tool calls too (e.g. `helm__get_today`,
  `test__echo`). Block, observe, and arg-rewrite all work identically to native tools.
  MCP tool names use the `server__tool` namespace; the existing `matcher` comma-list
  already handles them (e.g. `matcher = "helm__create_task"`).

**Shipped (Claude Code parity: blocking Stop, injected context, updatedInput, job wake-ups)**

See "Claude Code compatibility" below. In short: `Stop` hooks can keep the agent going,
`SessionStart` output is injected into the model's context, a CC `PreToolUse` hook's
`updatedInput` rewrites the tool call, and a finished background job wakes the model.

**Deferred**
- Per-hook environment templating beyond the stdin JSON.
- Other events: `notification`, `PostSessionCompact`.

## Claude Code compatibility

Hooks loaded from a Claude-Code-shaped `settings.json` (Forge's own `<config dir>/settings.json` and
`./.forge/settings.json`) run in `cc_compat` mode: they get the CC stdin payload (`session_id`,
`cwd`, `hook_event_name`, `tool_name`, `tool_input`, ...), start in the project directory with
`CLAUDE_PROJECT_DIR` set, and their output is read with CC rules (exit 2 blocks, stderr is the
reason). Bring your existing hooks over with `forge import claude --hooks` (opt-in; replaces only
the `hooks` key of the destination file, so it is safe to re-run). Matchers use CC tool names:
`Bash` matches `shell`, `Edit` matches `edit_file`, and so on.

| Event | CC behaviour implemented |
|---|---|
| `Stop` | Exit 2, or `{"decision":"block","reason":"..."}`, or `hookSpecificOutput.additionalContext` keeps the agent going: the text is sent back as the next instruction (`[stop hook] ...`). Input has `stop_hook_active` (true once the turn is already a continuation) and `last_assistant_message`. `{"continue":false}` ends the turn and wins over another hook's block. Consecutive blocks are capped by `stop_hook_max_blocks` (config, default 8, as in CC); the count resets when a continuation runs a tool. A loop-guard halt, hard guard, or turn deadline is never overridden. Fires after Forge's own completion gates (auto-review, autofix, plan approval). |
| `SubagentStop` | A block appends its reason to the `spawn_agents` result the parent model reads (CC re-drives the subagent itself; Forge has no subagent re-run, so the parent decides). |
| `SessionStart` | Plain stdout and `hookSpecificOutput.additionalContext` are added to the model's context as a system message. Native (non-CC) hooks still just print. `source` is always `startup` (no resume/clear/compact distinction yet). |
| `UserPromptSubmit` | `additionalContext` and plain stdout are appended to the prompt; `decision:"block"` / exit 2 blocks the turn. Forge appends to the prompt text (so it is visible in the transcript); CC keeps it out of sight. `sessionTitle` and `suppressOriginalPrompt` are ignored. |
| `PreToolUse` | `permissionDecision:"deny"` or exit 2 blocks. `updatedInput` replaces the tool arguments (CC argument names are translated: `Bash` `command`/`timeout` ms/`run_in_background`, `Edit`/`Write`/`Read` `file_path`, `old_string`, `new_string`), before Forge's permission check, workspace-path validation and schema validation. `permissionDecision:"allow"` does NOT skip Forge's permission gate; `ask`/`defer` are ignored. `additionalContext` becomes a model hint. This is what `rtk hook claude` emits. |
| `PostToolUse` | `additionalContext` becomes a model hint. A block degrades to a note. |

Not implemented: `type: "prompt" | "agent" | "http"` hooks (skipped at load), `async`, `if`,
`transcript_path` (always empty: Forge keeps its transcript in its store), `PostToolBatch`,
`TaskCompleted` and the other newer CC events. `forge run "<prompt>"` (one-shot) does not fire
`SessionStart` or `UserPromptSubmit`; the TUI, plain chat and daemon-hosted sessions do.

### Background job wake-ups

`shell` with `background:true` starts a job that outlives the call (see `shell_job`). When such a job
exits on its own, the session that started it is told, so the model does not have to poll:

- **Turn running:** a one-line notice (`[background job 4242 `npm run dev` exited 1]` plus the last
  lines of its log, capped) joins the conversation at the next model step, through the same inbox
  as a queued steer prompt (`inject_steers`).
- **Session idle:** the surface starts a turn with the notice(s), checked every ~5 s and right after
  each turn (TUI and daemon-hosted sessions, via the heartbeat check). Bursts within 1.5 s coalesce
  into one turn, and at most 6 automatic turns run per 10 minutes; further notices wait for the
  next user turn instead of being lost. At most 20 notices are queued (older ones are dropped and
  counted).
- Only jobs this session's model started are reported, and stopping a job yourself (`shell_job`
  `stop`) is not news. Jobs started before a restart, or by another session, are not reported.
- `forge mcp-serve` / bridged sessions and one-shot `forge run` do not auto-start turns (the process
  has no idle loop); the notice is still delivered if a turn is running.

## Non-goals
- Changing the agent loop, permission model, or tool contract. Hooks wrap `invoke_tool`; a
  block is reported exactly like a denied/errored tool call.

## 3. Acceptance criteria
```
Given a [[hooks]] entry event=pre_tool_use matcher="shell" command exits non-zero
When the model calls the shell tool
Then the tool does NOT run and the model receives "blocked by hook: <stderr>"

Given a pre_tool_use hook exits zero
When the matching tool is called
Then the tool runs normally and the hook's stdout is shown as a note

Given a post_tool_use hook
When a matching tool completes
Then the hook runs with {tool,args,result,ok} on stdin and its stdout is a note

Given a hook that runs longer than timeout_secs
When it fires
Then it is killed and noted; the turn proceeds (pre: not blocked)

Given no [[hooks]] configured
Then invoke_tool behaves byte-for-byte as before (no overhead)
```

## 4. Config example
```toml
[[hooks]]
event = "pre_tool_use"
matcher = "shell"            # absent or "*" = all tools; comma-list = several
command = "my-policy-check"  # reads {"tool","args"} on stdin; exit!=0 blocks

[[hooks]]
event = "post_tool_use"
matcher = "edit_file,write_file"
command = "graphify update . >/dev/null 2>&1 || true"
timeout_secs = 60
```

## 5. Design
`forge-config`: `HookConfig { event, matcher, command, timeout_secs }` + `HookEvent`; a
`Vec<HookConfig>` under `config.hooks`. `HookConfig::matches(tool)` does the name filter.

`forge-core::hooks::run_hooks(hooks, event, tool, payload)` filters to matching hooks, runs each
via `tokio::process::Command("sh","-c",cmd)` with the payload on stdin, `kill_on_drop(true)` +
`tokio::time::timeout`. Returns `HookOutcome { blocked: Option<String>, notes: Vec<String> }`.
`Session::invoke_tool` calls it before the tool (PreToolUse — short-circuits to a blocked
result) and after recording the result (PostToolUse — emits notes as warnings).

**Cross-platform:** hooks are POSIX `sh -c` only, like the shell tool (see known-issues.md).

## 6. Definition of done
- [x] `[[hooks]]` parses; `matches()` filter; default timeout.
- [x] PreToolUse non-zero blocks with the hook output as the reason; zero passes through.
- [x] PostToolUse runs with the result payload; stdout surfaced.
- [x] Timeout kills a wedged hook without hanging the turn.
- [x] Inert when unconfigured (existing tool tests unchanged).
- [x] Unit tests (runner) + an end-to-end test (a hook blocks a real `list_dir` call in a turn).
- [x] `cargo fmt` + `clippy -D warnings` clean.
