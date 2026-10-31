# Project actions and ports

One-click dev scripts for a project, and a view of what is listening.

## Config

Declare actions in the project's `.forge/config.toml`:

```toml
[[actions]]
name = "dev"
command = "npm run dev"
cwd = "web"      # optional, relative to the project; must stay inside it
icon = "play"    # optional, used by the graphical surfaces
```

An action is a shell command, so only a trusted project contributes them. In an untrusted
project the list is empty; `/run` says so and points at `forge trust` (or `FORGE_TRUST_PROJECT=1`
for a one-off). At most 24 actions load; names become lowercase slug ids and duplicates are
dropped. Parsing and validation live in `crates/forge-config/src/project_actions.rs`.

## TUI: `/run`

`/run` (aliases `/actions`, `/ports`) opens an overlay with two views, switched with Tab.

- **Actions**: each action with its state (idle, running with elapsed time, exited N, stopped).
  Enter runs an idle action and stops a running one, `r` restarts, `s` stops. Under the list sits
  the last lines of the focused action's output, ANSI colours kept, other escapes dropped.
- **Ports**: listening TCP ports of the actions and of anything running in the project directory,
  refreshed every 2 seconds while the overlay is open. `c` or Enter copies
  `http://localhost:<port>`, `o` opens it in the browser. Ports bound to all interfaces are
  marked `network`.

`/` filters either list. Esc clears the filter, then closes. While any action runs, the status
line shows a dot and the action name (or a count).

Each action runs as `$SHELL -lc <command>` in its own process group with stdin closed. Stopping
sends SIGTERM to the group and SIGKILL 1.5 seconds later, so a dev server's children go too.
Quitting the TUI kills every action it started. Driver: `crates/forge-cli/src/cli/commands/run/run_menu.rs`;
overlay: `crates/forge-tui/src/run_menu.rs`.

## Daemon, desktop, web and mobile

`forge serve` runs an action as a daemon terminal keyed `act-<id>` (`crates/forge-cli/src/serve_terminal/actions.rs`),
so output, attach and reconnect come from the terminal machinery. Routes (all need the `terminal` scope):

- `GET /api/terminals/actions?session=`, `GET /api/terminals/actions/running`
- `POST /api/terminals/actions/run` and `/stop` with `{session, action}`; the request never carries a command or directory
- `GET /api/ports?session=`: listening ports of the session's terminals, actions and worktree processes

Stop signals the whole process group (SIGTERM, SIGKILL after 1.5 s). The app has a Ports dock
(Layout menu, command palette, session header menu on phones) with Run/Stop rows, a listening-port
list (open in preview on desktop, open, copy URL), Run menus in the sidebar, palette entries
"Run: <name>" / "Stop: <name>", and a running indicator on sidebar and Fleet rows.

## Ports

`crates/forge-cli/src/ports.rs` finds listening sockets owned by a set of root processes and their
descendants, plus any process whose working directory is inside the project (Linux reads `/proc`;
other Unixes use `ps` and `lsof`; Windows reports nothing).

## Not done

- The authenticated reverse proxy `/p/<port>/` (stretch) is not built; a phone opens a port through the host's own address, which only works for ports bound to the network.
- The TUI does not attach to the action's terminal: output is a read-only tail, not a PTY, so
  programs that need a TTY (colour auto-detection, interactive prompts) behave as if piped.
- Actions started in the TUI are not visible to the daemon, and the reverse.
- No `icon` rendering in the TUI: the state glyph is shown instead.
