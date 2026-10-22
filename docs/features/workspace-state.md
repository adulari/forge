# Workspace state

Forge's runtime files for a workspace live in Forge's data directory, not in the user's project.
A project directory belongs to the user: a strict tree check, a `docker COPY`, `rsync` or
`grep -r` all see a stray `.forge/checkpoints/` as part of the project, and a benchmark that
compares the tree against an allow-list fails on it. Claude Code keeps its state outside the repo
for the same reason.

Code: `crates/forge-config/src/workspace_state.rs`. Callers: `forge-core/src/snapshot.rs`
(checkpoints), `forge-tools/src/evidence.rs` (evidence ledger), `forge-tools/src/shell/background.rs`
(background jobs), `forge-core/src/test_guard.rs` (test-edit patches).

## What lives where

| Kind | Where | Notes |
|---|---|---|
| Undo / rewind checkpoints | `<state>/checkpoints/<session>/<turn>/` | blobs and `manifest.json` per turn, see [rewind.md](rewind.md) |
| Evidence ledger | `<state>/evidence/<session>/` | screenshots, test output, previews, see [evidence-ledger.md](evidence-ledger.md) |
| Background jobs | `<state>/jobs/` | `<pid>.json`, `<pid>.log`, tunnel logs, see [shell-tool.md](shell-tool.md) |
| Test edits the guard took out | `<state>/test-edits/<n>.patch` | see [existing-test-guard.md](existing-test-guard.md) |

`<workspace>/.forge/` is for what the project authors: `AGENTS.md`, `config.toml`, `mcp.toml`,
`agents/`, `skills/`, `commands/`, `workflows/`. (A few Forge-written files still sit there:
`plans/`, `uploads/`, `worktrees/` and the TUI's `forge.log`.)

## The state folder

`<state>` is `<root>/<name>-<hash>`: the workspace's base name, so a person browsing the data
directory can tell folders apart, and a hash of its canonical path, so two checkouts called `app`
never share. The path is canonicalized first, so a symlink or `./sub/..` maps to the same folder.
A `workspace` file in it records the path.

`<root>`, first match wins:

1. `FORGE_STATE_DIR`, an explicit override. A CLI bridge passes its resolved root to
   `forge mcp-serve`, so the parent session and its tool child always use one folder.
2. `<FORGE_DB>.workspaces`, beside a scratch or operator-selected store (the same idea as the
   `.sweep-stamp` next to it), so tests and benchmarks that isolate the store never write into the
   real data directory.
3. `<data dir>/workspaces` (`~/.local/share/forge/workspaces` on Linux).
4. `<temp dir>/forge-workspaces` when no home directory resolves.

## Upgrading

State an earlier version wrote under `<workspace>/.forge/{checkpoints,evidence}/` is moved into the
state folder the first time a session (checkpoints) or ledger (evidence) opens that workspace, with
history and numbering intact, so `/undo` and `/rewind` still reach turns that predate the move.
Entries already present at the destination win. The old folder is removed once empty, together with
the `*` ignore file Forge put there and with `.forge/` itself when nothing else is in it. A project's
own `.forge/` content is never touched.

Background jobs are read from both places: jobs recorded under `.forge/jobs/` stay listed, tailed
and stoppable (a running one holds its log open, so it is not moved), and new jobs are only ever
written to the state folder. `read_file` may read this workspace's job logs even though they are
outside the project; checkpoints and other workspaces' state stay out of reach.

## Cleanup

The first time a process prepares a state folder it also removes, on a background thread, folders
whose workspace no longer exists and that nothing in them has changed for 30 days. The grace period is long on purpose: a workspace on
an unmounted drive looks gone too. Folders without a `workspace` marker are never touched.
