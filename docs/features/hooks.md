# Feature: pre/post tool-use shell hooks

> **Status: extended.** `[[hooks]]` config entries run a shell command (or an HTTP call, or a
> model judgement) around tool calls and session lifecycle events. `PreToolUse` blocks a call;
> `PostToolUse` / `PostToolUseFailure` observe; `UserPromptSubmit` can rewrite or block a user
> prompt; `SessionStart`/`SessionEnd` fire at session boundaries with Claude Code's `source` /
> `reason`. 23 of Claude Code's events are wired (see the table below). The hooks one event matches
> run concurrently. Wired into the direct tool path, the plain/TUI chat loops, daemon-hosted
> sessions and one-shot `forge run`.

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

**Shipped (Claude Code parity: new events, `http`, `if`, `async`, parallel hooks)**
- Events `PostToolUseFailure`, `PostToolBatch`, `PermissionDenied`, `SubagentStart`,
  `StopFailure`, `InstructionsLoaded`, `CwdChanged`, `WorktreeCreate`, `WorktreeRemove`,
  `UserPromptExpansion`, `PreModelSwitch` and `PostModelSwitch`, with Claude Code's payloads and
  matcher values (table below). Native names are the snake_case forms (`post_tool_use_failure`,
  `cwd_changed`, …).
- `type: "http"` handlers, the `if` permission-rule filter, `async: true` command hooks, and
  concurrent execution of the hooks one event matches (sections below).
- `forge run` fires `SessionStart`, `UserPromptExpansion` (for a `/command`), `UserPromptSubmit`
  and `SessionEnd`, as `claude -p` does. `SessionStart`'s `source` is `startup`, `resume`
  (`--resume`, `/resume`, the sessions picker), `clear` (`/new`) or `compact` (after compaction),
  and `SessionEnd`'s `reason` is `clear`, `resume`, `prompt_input_exit` (leaving chat) or `other`.

**Deferred**
- Per-hook environment templating beyond the stdin JSON.
- Events without a seam in Forge yet: `TaskCreated`/`TaskCompleted`, `ConfigChange`,
  `FileChanged`, `DirectoryAdded`, `Elicitation`/`ElicitationResult`, `Setup`, `MessageDisplay`,
  `TeammateIdle`; the `mcp_tool` handler type and `once`.

## Claude Code compatibility

Hooks loaded from a Claude-Code-shaped `settings.json` (Forge's own `<config dir>/settings.json` and
`./.forge/settings.json`; project hooks only once the project is trusted, see
[project-trust.md](project-trust.md)) run in `cc_compat` mode: they get the CC stdin payload (`session_id`,
`cwd`, `hook_event_name`, `tool_name`, `tool_input`, ...), start in the project directory with
`CLAUDE_PROJECT_DIR` set, and their output is read with CC rules (exit 2 blocks, stderr is the
reason). Bring your existing hooks over with `forge import claude --hooks` (opt-in; replaces only
the `hooks` key of the destination file, so it is safe to re-run). Matchers use CC tool names, and a matcher with regex syntax (`.*`, `^Read$`, `mcp__.*`) is treated as a case-insensitive regex tested against the Forge name and its CC alias (plain `A|B` stays an exact-name list):
`Bash` matches `shell`, `Edit` matches `edit_file`, and so on.

| Event | CC behaviour implemented |
|---|---|
| `Stop` | Exit 2, or `{"decision":"block","reason":"..."}`, or `hookSpecificOutput.additionalContext` keeps the agent going: the text is sent back as the next instruction (`[stop hook] ...`). Input has `stop_hook_active` (true once the turn is already a continuation) and `last_assistant_message`. `{"continue":false}` ends the turn and wins over another hook's block. Consecutive blocks are capped by `stop_hook_max_blocks` (config, default 8, as in CC); the count resets when a continuation runs a tool. A loop-guard halt, hard guard, or turn deadline is never overridden. Fires after Forge's own completion gates (auto-review, autofix, plan approval). |
| `SubagentStop` | A block appends its reason to the `spawn_agents` result the parent model reads (CC re-drives the subagent itself; Forge has no subagent re-run, so the parent decides). |
| `SessionStart` | Plain stdout and `hookSpecificOutput.additionalContext` are added to the model's context as a system message. Native (non-CC) hooks still just print. `source` is `startup`, `resume`, `clear` or `compact`, and the matcher filters on it (`"matcher": "resume"`). Fires in the TUI, plain chat, daemon-hosted sessions, `forge nl` and `forge run`. |
| `SessionEnd` | Observe-only. `reason`: `clear` (`/new`), `resume` (switching to another conversation), `prompt_input_exit` (leaving chat) or `other`; the matcher filters on it. |
| `InstructionsLoaded` | Fires after `SessionStart` once for each instruction file in context (`AGENTS.md`, `CLAUDE.md`, rules, imports, …) with `file_path`, `memory_type` (`User` / `Project` / `Local`) and `load_reason` (`session_start`, or `compact` after compaction), which the matcher filters. Observe-only. |
| `UserPromptExpansion` | A typed `/command` or skill that expands into a prompt (chat and `forge run "/cmd …"`). Payload: `expansion_type`, `command_name` (what the matcher filters), `command_args`, `command_source`, `prompt`. A block cancels the command; `additionalContext` joins it. |
| `UserPromptSubmit` | `additionalContext` and plain stdout are appended to the prompt; `decision:"block"` / exit 2 blocks the turn. Forge appends to the prompt text (so it is visible in the transcript); CC keeps it out of sight. `sessionTitle` and `suppressOriginalPrompt` are ignored. The session title is derived from the prompt text after hooks ran, so appended context can show up in it. |
| `PreToolUse` | `permissionDecision:"deny"` or exit 2 blocks. `updatedInput` replaces the tool arguments (CC argument names are translated: `Bash` `command`/`timeout` ms/`run_in_background`, `Edit`/`Write`/`Read` `file_path`, `old_string`, `new_string`), before Forge's permission check, workspace-path validation and schema validation. `permissionDecision:"allow"` does NOT skip Forge's permission gate; `ask`/`defer` are ignored. `additionalContext` becomes a model hint. This is what `rtk hook claude` emits. |
| `PostToolUse` | `additionalContext` becomes a model hint. A block (exit 2 / `decision:"block"`) is shown to the user and, as in CC, its reason is fed to the model (the tool already ran). A CC hook fires only after a call that ran and succeeded; a native hook still sees every call with its `ok` flag. |
| `PostToolUseFailure` | After a call that ran and failed (a tool error, or a `shell` command that exited non-zero). Payload adds `error`, `is_interrupt` and `tool_use_id`; the matcher and `if` filter the tool as for `PostToolUse`. `additionalContext` becomes a model hint next to the failed result. A refused call fires neither event. |
| `PostToolBatch` | Once per model step, after every call of the step resolved and before the next model request. Payload: `tool_calls` (each `tool_name`, `tool_input`, `tool_use_id`, `tool_response`). `additionalContext` joins the conversation; a block or `continue:false` ends the turn with the reason kept in the conversation. |
| `PermissionDenied` | Forge refused a call on its own: the permission mode forbids it (including an auto-mode classifier denial), a prompt no one was attached to answer, or the safety floor. A person saying no, a deny rule, a `PermissionRequest` hook's denial and a `PreToolUse` block do not fire it. Payload adds `reason` and `permission_mode`. `hookSpecificOutput.retry: true` adds a hint telling the model it may retry (not after a safety-floor refusal). |
| `SubagentStart` | Each child of `spawn_agents` as it is created, with `agent_id` (the child session) and `agent_type` (the agent name, which the matcher filters). It cannot stop the child; `additionalContext` is appended to the child's task. |
| `StopFailure` | A turn that ended on a provider error instead of an answer. `error` is Claude Code's type (`rate_limit`, `authentication_failed`, `billing_error`, `invalid_request`, `server_error`, `overloaded`, `model_not_found`, `unknown`) and the matcher filters on it; `error_details` and `last_assistant_message` carry the text. Observe-only. |
| `CwdChanged` | The `worktree` tool moved the session to another directory (`old_cwd`, `new_cwd`). Observe-only; a `systemMessage` is shown. |
| `WorktreeCreate` | Replaces `git worktree add` for the `worktree` tool's `create`: the hook gets `name`, makes the working copy however it likes (SVN, Perforce, a copy) and prints its directory as the last stdout line (an HTTP hook answers `hookSpecificOutput.worktreePath`). A failure, block or missing path fails the create; the path must be a directory, inside the workspace when `confine_to_workspace` is on. The session then switches into it. |
| `WorktreeRemove` | Removing a worktree a `WorktreeCreate` hook made (`exit` with `remove: true`, or `remove`) runs these hooks with `worktree_path` instead of git. Without one, Forge tries `git worktree remove --force` and reports anything left on disk. |
| `PreModelSwitch` | Before a model switch the user asked for (`/model <id>`, the model and rung pickers, a remote client's pick). Payload: `from_model`, `to_model`, `requested_model`, `source` (`command`, `picker`, `sdk`). The matcher tests the target model with or without its `provider::` prefix. Exit 2, `decision:"block"`, `permissionDecision:"deny"`, `continue:false`, a timeout, or `permissionDecision:"ask"` (Forge has no confirmation step on this path) keeps the current model and says why. Clearing a pin switches to `mesh`. |
| `PostModelSwitch` | After the pin changed. Plain stdout and `additionalContext` join the model's context. |
| `PermissionRequest` | Fires when a tool call would ask the user (the temper or an ask rule resolved to Ask). `hookSpecificOutput.decision.behavior: "allow"` approves it (an `updatedInput` replaces the arguments, re-checked against the workspace, the safety floor and deny rules); `"deny"` refuses it with `message` for the model, and `interrupt: true` also tells the model to stop the turn (Forge has no hard interrupt from a tool call). Exit 2 denies; no answer shows the prompt as usual. A deny from any hook wins over another's allow. Native hooks may print `{"action":"allow"}` / `{"action":"deny","reason":…}` and a non-zero exit denies. Unlike CC, it also fires in headless runs (where the prompt would otherwise be an automatic denial) and on the CLI bridge (`mcp-serve`). |

### `prompt` and `agent` hooks

Besides `type: "command"`, a hook can ask a model (`hooks_llm.rs`), in a CC `settings.json` or as a
native `[[hooks]]` entry with `type = "prompt"` / `"agent"`:

```json
{"hooks": {"Stop": [{"hooks": [
  {"type": "prompt", "prompt": "Did the agent finish everything the user asked? $ARGUMENTS"},
  {"type": "agent", "prompt": "Check that the tests in tests/ cover the change. $ARGUMENTS", "timeout": 120}
]}]}}
```

- `prompt` sends the instruction, with `$ARGUMENTS` replaced by the event's JSON (appended when the
  placeholder is absent), to the session's cheap side-call model (the auto-mode classifier's), or
  to `model` when the hook sets one.
- `agent` does the same through a read-only subagent: `read_file`, `search`, `glob` and `list_dir`,
  confined to the workspace, for up to 50 tool turns.
- The model must answer `{"ok": true}` or `{"ok": false, "reason": "…"}` (CC's older
  `{"decision":"approve"|"block"}` is accepted). `ok: false` behaves exactly like a command hook
  exiting 2 with `reason` on stderr: it blocks a `PreToolUse` call or a `UserPromptSubmit`, keeps a
  `Stop`/`SubagentStop` going, denies a `PermissionRequest`, and feeds a `PostToolUse` reason back.
  `ok: true` lets the event proceed (and approves a `PermissionRequest`).
- Timeouts default to 30 s (`prompt`) and 60 s (`agent`) in a CC `settings.json`, 30 s natively; a
  timeout, provider error or unparseable answer is reported as a hook error and does not block.
- Model hooks run where a session exists: the TUI, plain chat, `forge run`, `forge nl` and
  daemon-hosted sessions. On the CLI bridge (`mcp-serve`) and for `SessionStart`/`SessionEnd`
  they are skipped with a `hook error` note rather than guessed.
- Usage is recorded as a `hook` side call.

### `http` hooks

`type: "http"` POSTs the event JSON (the same body a command hook reads on stdin) to `url`:

```json
{"hooks": {"PreToolUse": [{"matcher": "Bash", "hooks": [
  {"type": "http", "url": "http://localhost:8080/policy", "timeout": 10,
   "headers": {"Authorization": "Bearer $POLICY_TOKEN"}, "allowedEnvVars": ["POLICY_TOKEN"]}
]}]}}
```

Natively: `type = "http"`, `url`, a `[hooks.headers]` table and `allowed_env_vars`. `$VAR` /
`${VAR}` in a header expand only for variables listed in `allowedEnvVars`; any other reference
becomes empty, so a header cannot leak a variable the configuration did not name. A 2xx response
with an empty body is a silent success; a 2xx JSON object is read exactly like a command hook's
stdout (`decision`, `hookSpecificOutput`, …). A non-2xx status, a connection failure, a timeout or
a body that is not a JSON object is a non-blocking error (noted, never a block), as in Claude Code:
to block, answer 2xx with a decision. Timeouts default to 600 s in a CC `settings.json` (30 s for
`UserPromptSubmit` and the model-switch events) and 30 s natively.

### `if`

A hook on a tool event (`PreToolUse`, `PostToolUse`, `PostToolUseFailure`, `PermissionRequest`,
`PermissionDenied`) may carry `if`: one permission rule the call must also match, in Claude Code's
syntax (`Bash(git *)`, `Edit(*.ts)`, `Read(//etc/**)`, `WebFetch(domain:docs.rs)`, `mcp__github`).
Forge tool names work too (`shell(cargo *)`). A `Bash` rule is tested against every command of a
compound line (`cd x && git push` matches `Bash(git push*)`), and a line Forge cannot split
confidently runs the hook anyway, so `if` narrows a hook, it never replaces a deny rule. On any
other event a hook that carries `if` does not run, as in Claude Code.

### `async` command hooks

`"async": true` on a `command` hook starts it in the background: the event never waits for it and
nothing it prints can block. When it exits, the `additionalContext` and `systemMessage` of its
JSON answer join the model's context at the start of the session's next turn.

### Parallel execution

All hooks one event matches start together (at most 12 at once) and their results are applied in
declaration order, so the outcome does not depend on which finished first: two 1 s hooks on the
same event take about 1 s, not 2 s. The first block in declaration order wins.

Not implemented: `type: "mcp_tool"`, `once`, `transcript_path` (always empty: Forge keeps its
transcript in its store), and the CC events listed under Deferred above.

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
