# Move a Home session into the repo it keeps working in

A session started from the home directory (or any folder that is not a git repository) gets none
of a project's instructions, Lattice index, change queue, trust or worktree option. When the agent
keeps running `cd <repo> && ...` there, Forge offers to move the session into that repository.

## When it appears

- The session's directory is your home directory, or is not inside a git repository.
- At least 3 `shell` calls of the session begin with `cd` / `pushd` (also in `cd a && cd b`, `;`
  and `||` chains; `~`, `$HOME` and relative paths resolve) into the same git repository.
- The repository is below your home directory and is not the home directory itself.
- The session is idle. A dismissed repository is not offered again in that session.

The detector is `forge_tui::project_offer` (pure, unit-tested); the `App` counts calls as their
`ToolStart` events arrive, so the terminal UI and daemon-hosted sessions behave identically.

## Surfaces

- **TUI**: a card above the composer (it shares the update card's slot): `m` moves the session,
  `d` dismisses. Moving is the same switch `/project` performs (`Session::switch_project`).
- **Desktop and phone**: `ProjectOfferCard` above the composer, from the snapshot field
  `project_suggestion: {path, name, calls}`. Buttons send `project_move` / `project_dismiss`
  with the offered `path`; a stale path is dropped with a note. A moved daemon session reports
  the new `cwd` in its snapshot and in `GET /api/sessions`.

Try it offline: `mock:shell cd code/forge && ls` sent three times to a `--mock` session.
