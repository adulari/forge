# Import conversations

Bring the Claude Code and Codex conversations already on this machine into Forge, and keep going
where they stopped. The originals are only ever read.

| Agent | Where it is read from | What comes over |
|---|---|---|
| Claude Code | `~/.claude/projects/*/<id>.jsonl` (or `$CLAUDE_CONFIG_DIR`); interactive sessions only, never Forge's own bridge runs | the active chain with its tool calls and results; history before the last compaction stays in replay but out of the model context; pinned to `claude-cli::<model>` |
| Codex | `~/.codex/sessions/YYYY/MM/DD/rollout-*-<id>.jsonl` (or `$CODEX_HOME`) | user and assistant text, tool calls with their output (reasoning is encrypted and dropped); pinned to `codex-cli::<model>` |

A scan marks what is already in Forge. Claude Code sessions are tracked by the same link `forge
switch` uses; Codex imports are remembered in `<forge.db>.codex-imports.json`. A deleted session is
importable again.

## Surfaces

- **Desktop, web and phone**: Settings > Import conversations (`/import` route, also in the command
  palette and the first-run import step). Search, agent and folder filters, select all, a
  plural-aware result toast, and a project picker for conversations whose folder is not on the host.
- **Terminal chat**: `/import` opens a picker. Type to search, Tab changes the agent, Space selects,
  Right selects all and Left clears, Enter imports the selection (or the focused row).
- **Command line**: `forge import scan [--agent claude|codex] [--json]`,
  `forge import sessions <id>... | --all [--agent A] [--cwd DIR]`,
  `forge import codex-session <id|path>`, and the existing `forge import claude-session`.
- **Daemon**: `GET /api/import/scan` and `POST /api/import` with `{items: [{agent, id}], fallback_cwd}`.
  A request names a conversation, never a path; the daemon resolves it against its own scan.

Conversations whose folder no longer exists open in the fallback project (the terminal chat's
directory, the home directory, or the one picked in the app).

Over Forge Anywhere the two routes are not bridged yet: the relay protocol has a fixed route list.
Use the app on the same network, or the terminal.
