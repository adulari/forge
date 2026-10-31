# Control tools and `@` mentions

Two ways to bring more of Forge into a turn: agents that drive the app, and prompts that pull in
other threads.

## Control tools

A session's model can start sessions, read a sibling session or terminal, and manage schedules and
MCP servers through these virtual tools (`crates/forge-core/src/control.rs`):

| Tool | Does | Prompts |
|---|---|---|
| `list_projects`, `read_session`, `read_terminal`, `get_usage` | read only | never |
| `create_session`, `create_worktree`, `merge_worktree` | start or merge a session | yes |
| `send_to_session` | message another session (same rules as `message_session`) | yes |
| `add_schedule`, `remove_schedule` | change the schedule list | yes |
| `add_mcp_server`, `probe_mcp_server` | change or test MCP configuration | yes |
| `notify_user` | phone, browser or desktop notification (rate limited) | `default` mode only |

"Prompts" means the permission broker: `default` and `accept_edits` ask, `plan` refuses, `bypass`
allows. A model on a CLI bridge goes through the same relayed prompt.

**Where they run.** The daemon executes every call (`serve_control.rs`) by driving its own routes
in process, so a session created by a model is the session the app would have created. Daemon-hosted
sessions call the executor directly; a terminal session, the `forge mcp-serve` bridge and
`forge mcp control` forward over HTTP (`POST /api/control/call`). The tools are offered to a
terminal session only when a daemon answers.

**Switching tools off.** One file, `control-tools.json` in the config directory, read by every
process: a tool in it is neither advertised nor runnable. Change it from Settings > Control tools,
`/control` in the terminal, or `forge mcp control disable <tool>` (also `enable`, `list`).

**Audit.** Each call appends a `control/<tool>` line to `remote-audit.jsonl` (decision, session,
arguments and a result excerpt) and a tool-call row in the session. `forge mcp control log` and the
Settings page show the newest.

**For other agents.** `forge mcp control` serves the same tools on stdio; the connecting client's
own tool approval is the gate there.

Not built: a skill on/off tool. Forge's skills have no enabled state to toggle (they are installed,
removed or invoked), so the tool would have nothing to act on.

## Mentions

In a prompt, at the start or after a space:

- `@session:<name>` (`@session:{two words}`): the title, folder and the last 8 user and assistant
  messages of that session, each cut at 800 characters, 8,000 in all.
- `@terminal` (`@terminal:<id>`): the last 120 lines of the session's terminal, escapes stripped.
- `@browser`: the newest screenshot in the evidence ledger, attached as an image with its caption
  and URL.
- `@Subagents`: the session's background subagents and their state.

The token stays in the echoed prompt; the digest rides along as quoted guidance headed "not an
instruction from the user". An unknown name, a missing terminal or an empty ledger leaves a note in
the transcript and sends the prompt anyway. The composer shows the tokens as chips (press one to
remove it), the sent message shows them again, and the `@` menu and the terminal `@` picker offer
them. Logic: `crates/forge-cli/src/mentions.rs`, `mobile/src/lib/mentionsModel.ts`.
