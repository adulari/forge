# Feature: Claude Code parity (statusline, notifications, language, auto mode, wakeups, agent types, commit policy)

Small gaps people hit when moving from Claude Code to Forge.

## Statusline command

`[statusline] command = "~/.claude/statusline.sh"` runs a Claude-Code-style statusline script.
Forge pipes session JSON to its stdin and shows the first stdout line as one extra row under the
built-in statusline. SGR colours in the output are honoured; every other escape sequence is dropped.

`forge import claude` copies `statusLine.command` from `~/.claude/settings.json` (or the project
`.claude/settings.json` with `--project`) into this key. An existing `[statusline]` table is never
edited; the importer says so and leaves the key for you to set.

Payload fields: `session_id`, `transcript_path` (empty, Forge keeps transcripts in its store),
`cwd`, `model {id, display_name}`, `workspace {current_dir, project_dir}`, `version`,
`output_style`, `cost {total_cost_usd, total_duration_ms, total_lines_added, total_lines_removed}`
(line counts are always 0), `context_window {total_input_tokens, total_output_tokens,
context_window_size, used_percentage, remaining_percentage, current_usage}`, `exceeds_200k_tokens`.

The script runs detached: at most one at a time, no more than once per 300 ms, only when the
payload changed (the ticking duration does not count), killed after 3 s. A failing or slow script
leaves the previous row on screen; it never blocks drawing.

## Notifications

```toml
[notifications]
enabled = false        # off by default
desktop = true         # notify-send (Linux) / osascript (macOS)
bell = true            # terminal BEL
min_turn_secs = 15     # a finished turn only notifies if it ran this long
when_focused = false   # also notify while the terminal has focus
```

Fires when a turn finishes (after `min_turn_secs`) and when a permission prompt or `ask_user`
question appears. By default only while the terminal is unfocused; set `when_focused = true` for
terminals/tmux setups that do not report focus events.

## Reply language

The base system prompt tells the model to answer in the language of the user's latest message.
This stops models such as Kimi from drifting into another language mid-session. Bridged Claude and
Codex turns receive the same prompt, since system messages are flattened into the bridge transcript.

## Auto permission mode (`--mode auto`)

Forge already had Tempers (Read-only / Ask / Auto-edit / Full). **Auto-edit** auto-approved edits
*and* shell, relying on the builtin deny rules alone, so there was no "proceed unless risky"
posture. (Auto-edit has since been narrowed: it asks for shell that deletes, moves or writes outside
the workspace, see temper-modes.md.) The new **Auto** temper (`PermissionMode::Auto`, key `auto`) fills it:

| call | outcome |
| --- | --- |
| read-only tools | allow |
| edits inside the workspace (or `/tmp`) | allow |
| shell that is not risky | allow |
| network GET / HEAD | allow |
| builtin-deny match (`rm -rf /`, `.env`, `~/.ssh`, ...) | **deny** (the floor still beats every mode) |
| risky shell or write | **ask** |
| external MCP tool | ask |

Risky means: `rm -r/-f`, `git reset --hard`, force-push or remote branch delete, `git clean`,
`git checkout -- .`, `git branch -D`, `DROP`/`TRUNCATE` SQL, `dd of=`, `mkfs`, recursive
`chmod`/`chown`, `sudo`, `kill`, container/cluster/terraform deletes, package `publish`, uploads
(`curl -d/-F/-T/-X POST`, `scp`, `ssh`, `nc`, `rsync` to a remote), pipe-to-shell, writes (redirects,
`cp`/`mv`/`tee`/`sed -i`, write tools) outside the workspace or via `~`/`..`, and reads of credential
paths or secret environment variables. The classifier is `permission/auto.rs` (`auto_risk`); it is a
heuristic floor like the denylist, not a sandbox. Explicit `allow`/`ask`/`deny` rules are resolved
before it, so a user rule can lift or tighten any single command. When auto asks, a warning line
names the reason.

Selectable with `forge run --mode auto`, `permission_mode = "auto"` in config, `/mode`, and the
SHIFT+TAB cycle (Ask, Auto-edit, Auto, Read-only). The bridge treats an ask as a
deny (no interactive surface), as for every mode; a subagent's ask goes to the parent's prompt when
there is one (see `subagent-orchestration.md`).

## `schedule_wakeup`

Survey: `forge schedule` (OS timers, fresh process), `/heartbeat` and `manage_heartbeats`
(recurring prompts into the live session) and background-job exit wakes already existed. Missing
was Claude Code's one-shot "resume me in N seconds". `schedule_wakeup(delay_seconds, prompt,
reason?)` clamps the delay to 60-3600 s and re-enters the same session with the prompt once it is
idle. It is stored as a one-shot agent heartbeat under the reserved label `wakeup` (no
migration), deleted when delivered; calling it again replaces the pending one. Advertised on the
direct path and through the CLI bridge (`mcp_serve`, via the shared store). Like heartbeats it
fires only while the session is open (TUI or daemon), not in a one-shot `forge run` that exits.

## Subagent types

`.forge/agents/*.md` (name, description, tools, tier) already loaded. Added: `.claude/agents/`
(project and `~/.claude/agents`) and `~/.forge/agents` are merged in, later layers winning
(`~/.claude`, `~/.forge`, project `.claude`, project `agents_dir`). Claude `tools: Read, Grep, Bash`
lists translate to Forge tool names (unknown Claude tools are dropped), `model:` maps `haiku` /
`sonnet` / `opus` to a tier and `provider::model` to a hard pin, and `spawn_agents` now lists
each type with its description and tools and accepts `subagent_type` as an alias of `agent`.

## `[git] commit_policy`

`auto` (default: `unit` in an interactive session, `request` when that session's own commit would
stop at a permission prompt, `end` for a headless one-shot run: `forge run`, `forge run -p`,
`forge bench`), `unit` (commit each verified unit), `end` (one commit when the task is complete),
`request` (commit only when the user asks) or `never` (leave changes in the tree). It swaps the
system prompt's version-control paragraph (`commit_policy.rs`) and silences the git-hygiene
reminders for anything but `unit`. The never-push rule is kept by every policy. Setting any value
explicitly overrides the `auto` choice. See `commit-discipline.md` for why `request` exists.

## Logged-in browser attach

Built: `[browser] attach = "http://127.0.0.1:9222"` (alias `cdp_url`, env `FORGE_BROWSER_CDP`) makes
the `browser` tool's `open` attach to an already-running Chromium-family browser instead of
launching one. It opens its own tab, never adopts or closes yours (only `/json/close/<its tab>`; no
`Browser.close`/`Target.closeTarget`), and `forge browser attach` starts a browser on a dedicated
persistent profile for you to log into once. Details, safety model and the Chrome 136 default-profile
restriction: [browser.md](browser.md#attach-to-your-logged-in-browser). Still not built: reading
cookies out of your everyday profile, or bridging Claude's Chrome extension over native messaging.

## `claude -p` flags on `forge run`

`forge run` accepts Claude Code's print-mode flags, so scripts written for `claude -p` run
unchanged. Print-mode flags live in `crates/forge-cli/src/cli/args/claude_compat.rs`; the flags
that shape the session itself live in `args/session_flags.rs` (`SessionFlags`, a clap struct
`forge chat` can flatten too). Config-level flags are applied by `run/claude_flags.rs`, per-session
ones by `run/print_run.rs`.

| Flag | Effect |
|---|---|
| `-p`, `--print` | print mode: stdout carries only the final answer (one line per turn, nothing on failure). The trace (session, routing, narration, tool calls, cost) goes to stderr when stderr is a terminal, or with `--verbose`; errors always reach stderr. Without `-p`, `forge run` streams its line view to a terminal and behaves as `-p` whenever stdout is a pipe or a file (and then stays in this process instead of being handed to a `forge serve` daemon); `/goal` keeps its progress lines |
| `--verbose` | with text output, write the trace to stderr even when it is not a terminal; with `stream-json`, add `system/status` events (see [stream-json](#stream-json-output)) |
| `--quiet` | print mode with no trace at all, not even at a terminal; errors still reach stderr. Conflicts with `--verbose`; accepted and ignored with `json` and `stream-json` |
| `--output-format text\|json\|stream-json` | see [stream-json](#stream-json-output) below |
| `--include-partial-messages` | with `stream-json`, also emit token-level `stream_event` lines |
| `--input-format text\|stream-json` | `stream-json`: one JSON user message per stdin line (Claude Code's SDK shape, string or text-block content), each run as a turn of the same session with its own `result`; `control_request` `initialize` is answered with success, other control requests with an error |
| `--system-prompt TEXT`, `--system-prompt-file FILE` | replace Forge's base coding-agent prompt for every model, over any configured `system_prompt_overrides` (the environment block stays). Bridged CLIs (`claude-cli::`, `codex-cli::`) still run on their own built-in prompt underneath |
| `--append-system-prompt TEXT` (alias of `--system`), `--append-system-prompt-file FILE` | standing system messages, kept out of mesh classification |
| `--add-dir DIR...` | adds each (existing) directory to `[tools] extra_roots` for this run and tells the model about it |
| `--effort low\|medium\|high\|xhigh\|max\|ultracode` | the `/effort` ceiling for the session; `max` = `xhigh`, `ultracode` = `whitehot` |
| `--max-budget-usd N` | stops the run once its priced spend reaches N dollars (a spend ceiling armed per turn, `run_budget.rs`); the result is `error_max_budget_usd` and the run exits 1. A `/goal` loop keeps its own `--max-tokens`/`--max-minutes` budget instead |
| `--session-id UUID` | creates the session with this id (refused when the id is in use or not a UUID) |
| `-n`, `--name NAME` | session title, shown in `forge sessions`; `--resume NAME` resumes the newest session with that name |
| `--no-session-persistence` | the session is deleted when the run ends and cannot be resumed; its usage moves to a hidden archived ledger session so `forge stats` and the day/week/month caps still count it |
| `--mcp-config FILE\|JSON...` | MCP servers for this run in Claude Code's `.mcp.json` shape (`mcpServers`, stdio `command`/`args`/`env`, `http`/`sse` `url`/`headers`, `${VAR}` and `${VAR:-default}` expansion), added to the configured servers; nothing is written to disk |
| `--strict-mcp-config` | use only the `--mcp-config` servers (configured servers and Forge's own self-MCP server are dropped) |
| `--setting-sources user,project,local` | which config layers load; `project` and `local` both mean the project's `.forge/` (config.toml, mcp.toml, settings.json); an empty list loads built-in defaults only. Instruction files follow it too: without `user` the global ones are skipped, without `project`/`local` the project ones ([instruction-files.md](instruction-files.md)) |
| `--fallback-model A,B` | models tried, in order, when the routed model is unavailable, before the mesh's own chain; with `--model` they are the only models the run may fall to |
| `--continue`, `--resume`, `--model` | as before; a resumed run always stays in this process. `--continue` takes the most recent session of the current directory, as `claude -c` does; `--all-dirs` takes the most recent of any |
| `--permission-mode default\|acceptEdits\|bypassPermissions\|plan\|auto\|dontAsk` | same as `--mode`; conflicts with it. `dontAsk` is `default` that never prompts, even at a terminal: anything not pre-approved is denied (what a run without a terminal always does) |
| `--dangerously-skip-permissions` | same as `--mode bypass` |
| `--max-turns N` | caps model steps for the run (`mesh.max_steps` and `max_steps_unattended`); without it there is no step cap, only the [progress supervisor](progress-supervision.md) |
| `--allowedTools`, `--disallowedTools` | `"Bash(git log:*) Read"` becomes `[[permissions.rules]]` on `shell` / `read_file` (`:*` becomes ` *`); deny wins; space- or comma-separated, repeat the flag for more |

`-p` and every session-shaping flag keep the run in this process instead of handing it to a
running `forge serve` daemon, which could not honour them.

Not supported, and rejected by clap rather than ignored: `--fork-session`, `--json-schema`,
`--settings`, `--tools`, `--agents`, `--agent`, `--worktree`, `--bare`.
`--allowedTools a b c` with several space-separated arguments is not accepted: pass one quoted
string or repeat the flag.

Piped stdin is appended to the prompt (`cat log | forge run -p "explain"`). A pipe that stays open
and silent does not hang the run: with a prompt argument the read gives up after 3 s and the run
goes ahead without it (a warning on stderr); with no prompt argument it waits 30 s for the prompt.
A terminal stdin is never read, and neither is stdin under `--input-format stream-json`.

### stream-json output

Shapes follow `claude -p --output-format stream-json` (`crates/forge-tui/src/stream_json.rs`):

- `system/init` first, with `session_id`, `cwd`, `model` (the pin, or `auto`), `permissionMode`,
  `tools`, `mcp_servers` (`name`, `status`), `slash_commands`, `skills` and `forge_version`.
- One `assistant` event per finished message (one model call): whole text and thinking blocks,
  `tool_use` blocks on the same message id, `id`, `model` and the call's `usage`. Deltas appear only
  as `stream_event` lines (`message_start`, `content_block_start/delta/stop`, `message_delta`,
  `message_stop`) under `--include-partial-messages`.
- `user` events carry `tool_result` blocks (the tool's full output when it has one).
- `result` closes each turn with `subtype` (`success`, `error_max_turns`, `error_max_budget_usd`,
  `error_during_execution`), `is_error`, `result`, `duration_ms`, `duration_api_ms`, `num_turns`
  (model calls), `total_cost_usd` and `usage` for that turn only (restored totals of a resumed
  session are the baseline; `input_tokens` excludes cache reads, as Anthropic reports it),
  `permission_denials` and `session_id`.
  `usage` is Anthropic's shape: `input_tokens` excludes both cache counts, so `input_tokens +
  cache_read_input_tokens + cache_creation_input_tokens` is the total sent. `modelUsage` has one
  object per model that served calls during the turn (side calls included), keyed by Forge's
  model id: `inputTokens`, `outputTokens`, `cacheReadInputTokens`, `cacheCreationInputTokens`,
  `webSearchRequests` (always 0) and `costUSD`. Cache writes and the split are read from the
  session's store (`usage_source.rs`); a run with no store reader falls back to the turn's cost
  events under the model it was routed to, with cache writes at 0. A run that fails before its
  first turn ends with the same fields zeroed.
- Every event carries `session_id`; `assistant`, `user` and `stream_event` carry
  `parent_tool_use_id: null` and a `uuid`. Forge adds `system/routing`, `system/usage` (the running
  session total, sent once per change), `system/warning`, `system/error` and `system/tool_output`,
  which Claude Code consumers ignore.
- `--verbose` adds one `system/status` (`{"status": "requesting"}`) before every model call, as
  Claude Code's verbose stream does. Claude Code refuses `stream-json` without `--verbose`; Forge
  does not need it.

`--output-format json` prints the same `result` object once.

## Output streams

A write to a closed stdout (`forge mcp tools | head`) exits 141 quietly; any other stdout/stderr
write failure (`> /dev/full`) prints one `error:` line and exits 1. Neither changes how the daemon
handles SIGPIPE.

## Entry points

- Bare `forge` on an interactive terminal starts `forge chat`, as bare `claude` starts a session,
  once a provider is configured. With no provider, or when stdin or stdout is not a terminal (a
  pipe, CI, `TERM=dumb`), it prints the short setup or common-commands panel and exits 0 as before
  (`first_run.rs`).
- `forge chat "prompt"` sends the prompt as the first message as soon as the session is up and
  leaves the chat open, like `claude "prompt"`. It dispatches like typed input, so a `/command`
  works too; when the automatic project setup starts first, the prompt queues behind it. With
  `--plain` the prompt runs first and stdin takes over afterwards.
- `forge mcp add` takes `claude mcp add` syntax: `forge mcp add [options] <name> <commandOrUrl>
  [args...]` with `-t/--transport`, `-H/--header "Name: value"` (Forge's older `Name=value` still
  works; the first separator wins, so `Bearer abc==` keeps its padding), `-e/--env` and `-s/--scope`.
  A URL target with no transport means `http`, where Claude Code would try to spawn the URL as a
  stdio command. `--url` and `-- <command>` still work. `forge mcp list` (alias `ls`) is bare
  `forge mcp`. `forge mcp add-json <name> '<json>'` takes one server's `.mcp.json` entry; a token in
  its env or headers goes to the keyring, as with `forge mcp import`.
  Not mapped: `--client-id`, `--client-secret`, `--callback-port` (OAuth client settings live in
  `mcp.toml`, see mcp-client.md), `add-from-claude-desktop` (`forge mcp import` scans Claude
  Desktop), `reset-project-choices`, and `serve` (`forge mcp agent`).

## Slash commands and keys from Claude Code

| Claude Code | Forge |
|---|---|
| `/clear` (`/reset`, `/new`) | starts a new conversation with an empty context, same as `/new`. Clearing only the screen is now `/cls` (it was `/clear` before v2.21) |
| `/context` | the next request's estimated size by category (system prompt, tool definitions, project instructions, memory/goal/tasks, your messages, replies, tool results, harness notes), with a bar and percentages when the model's window is known, plus the provider's count for the last request. A text breakdown, not CC's coloured grid |
| `/diff [path]` | the uncommitted changes against `HEAD`, staged or not, with untracked files listed: the searchable full-screen viewer with diff colours, scrollback lines in `--inline` mode. Runs git off the render loop, so it works while a turn runs |
| `/permissions` (`/allowed-tools`) | the temper and every allow/ask/deny rule with where it came from (config, an always-allow answer, this session); built-in safety rules are counted. `/permissions allow\|ask\|deny Bash(git log:*) Read` adds rules for this session in CC syntax. No interactive editor and no removal: persistent rules stay in `[[permissions.rules]]` |
| `/mcp reconnect [server]` | drops and re-establishes one configured server, or all of them, in the background; `/mcp` still lists |
| `/add-dir <path>` | the file tools may work in that directory for the rest of the session exactly as with a `[tools] extra_roots` entry (reachable when confined; inside the project for Auto-edit). Session-only, and bridged `claude-cli::`/`codex-cli::` turns keep the roots their tool server started with |
| `/doctor` | `forge doctor`'s local checks (config, credentials, model health, daemon, Anywhere, language servers, Ollama, store, environment) in the full-screen viewer. Provider reachability and bridge round-trips stay with the CLI command, since they make network calls and spend quota; nothing is auto-fixed |
| `/rename <title>` | names the session in `/sessions`, the resume picker and the apps; bare `/rename` shows the current name. Works while a turn runs: it writes the title straight to the store instead of waiting for the turn |
| `/review`, `/code-review`, `/security-review` | built-in commands: a correctness review of the uncommitted diff, a PR number, a branch or a path (`--fix` applies verified fixes), and a security review of the branch against the remote default branch. A user or project command with the same name replaces them; CC's effort levels and `--comment` are not supported |
| Ctrl+L | repaints the screen. Compaction moved from Ctrl+L to Alt+L (`[keybinds.binds] compact`); a config that still binds `compact` to Ctrl+L is moved on load while `redraw` keeps Ctrl+L |
