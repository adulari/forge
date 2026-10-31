# TUI session strip, sidebar, diff overlay and background shells

Four additions to the chat TUI that help when several sessions are running or a lot of files
changed. All of them live in `forge-tui` as pure views; `forge-cli`'s `run/panes.rs` is the host
that reads git, the filesystem and the daemon.

## Session strip

`/strip [on|off]` (or `[tui] session_strip = true`) puts one row above the chat that lists the
daemon's sessions: `1 ⠋ fix login   2 ● docs   3 · refactor`. A spinner is a busy session, a dot is
one waiting on you (a permission or a question), a faint `·` is idle, and this chat's own session is
highlighted. **Alt+1 to Alt+9** switch: the TUI hands the terminal to `forge attach <id>` and takes it
back when you detach. The rows come from `GET /api/sessions` and are read again whenever `/ws/fleet`
says the fleet moved. With no daemon the strip says so instead of hiding.

## Sidebar

`Ctrl+B`, `/sidebar [on|off]` or `[tui] sidebar = true` opens a 34-column sidebar on full-screen
terminals at least 120 columns wide: model and `folder:branch`, the context gauge, spend, one dot per
MCP server (green connected, amber reconnecting or slow, red failed or unauthorized), and the task
list. It replaces the details rail (it already carries the tasks).

## Diff overlay

`/diff` with no path opens a full-screen review of the changes (full-screen chat; the inline chat
keeps its pager). `/diff <path>` keeps printing the text diff. Two sources, switched with `d`: **working tree vs HEAD** (tracked changes, renames, and
untracked text files shown as additions) and **this turn** (what the agent changed since your last
prompt; long hunks are capped at the lines the remote diff card keeps, and the file says so).

| Key | Action |
|---|---|
| `j` / `k`, arrows, PageUp/PageDown, `g` / `G` | scroll |
| `]` / `[` | next / previous hunk |
| `n` / `p` | next / previous file |
| `v` | unified or side by side (side by side needs 70 columns) |
| `d` | switch source |
| `m` | mark the file reviewed and move to the next unreviewed one |
| `r` | read git again |
| `Esc` / `q` | close |

A reviewed mark belongs to the file's content: if the file changes again it is no longer marked.
From 100 columns a file list with the marks sits beside the diff.

## Background shells

`/jobs` lists the `shell{background:true}` jobs of this workspace with their status and age. Enter
shows the log tail, `x` stops the job's process group, `r` refreshes. It works in inline mode too,
where the live region has no room for a panel.

## Already there, and what is not here

- Find in the transcript is `Ctrl+F` (full-screen mode). Inline, the terminal owns the scrollback and its own search does the job.
- The subagent drill-in is the agents panel (`Ctrl+O`, Enter on a row).

## Other surfaces

Desktop, web and mobile already have these jobs as screens: the Fleet sidebar lists every session with
its busy and waiting state, the git panel reviews diffs, and Settings shows usage and MCP health. A strip,
a sidebar column and a key-driven overlay are terminal conventions with no counterpart that
would add anything there, so this package ships on the TUI only.
