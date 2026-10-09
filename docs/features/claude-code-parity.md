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

Forge already had Tempers (Read-only / Ask / Auto-edit / Full). **Auto-edit** auto-approves edits
*and* shell, relying on the builtin deny rules alone, so there was no "proceed unless risky"
posture. The new **Auto** temper (`PermissionMode::Auto`, key `auto`) fills it:

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
SHIFT+TAB cycle (Ask, Auto-edit, Auto, Read-only). Subagents and the bridge treat an ask as a
deny (no interactive surface), as for every mode.

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

`unit` (default, unchanged), `end` (one commit when the task is complete) or `never` (leave
changes in the tree). It swaps the system prompt's version-control paragraph
(`commit_policy.rs`) and silences the git-hygiene reminders for anything but `unit`. The
never-push rule is kept by every policy.

## Logged-in browser attach

Built: `[browser] attach = "http://127.0.0.1:9222"` (alias `cdp_url`, env `FORGE_BROWSER_CDP`) makes
the `browser` tool's `open` attach to an already-running Chromium-family browser instead of
launching one. It opens its own tab, never adopts or closes yours (only `/json/close/<its tab>`; no
`Browser.close`/`Target.closeTarget`), and `forge browser attach` starts a browser on a dedicated
persistent profile for you to log into once. Details, safety model and the Chrome 136 default-profile
restriction: [browser.md](browser.md#attach-to-your-logged-in-browser). Still not built: reading
cookies out of your everyday profile, or bridging Claude's Chrome extension over native messaging.
