# Feature: Claude Code parity (statusline command, notifications, reply language)

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
