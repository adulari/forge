# Feature: session worktrees — the `worktree` tool

> Status: **SHIPPED** (2026-10-14). Code: `crates/forge-core/src/worktree_ops.rs` (request parsing
> and the git work, shared by both paths), `crates/forge-core/src/session_worktree.rs` (the direct
> path: permission gate, the session switch, following a bridge), `crates/forge-cli/src/mcp_serve/worktree.rs`
> (the CLI bridge). Tests: `crates/forge-core/src/tests/worktree_ops.rs`,
> `crates/forge-core/src/tests/session_worktree.rs`.

## 1. Problem

A session could create a git worktree only through `shell`. Its workspace stayed the main checkout,
so the shell's default cwd, relative paths and the file tools all kept resolving there. Measured
live with `inferhub::glm-5.3-flash` before this shipped:

- asked to work in a worktree, the model wrote every file with `cat > ../wt/file <<EOF` through
  `shell` and prefixed every command with `cd ../wt`; it never used a file tool in the worktree;
- in another run it guessed the worktree's absolute path wrong and `append_file` silently created
  a stray `calc.py` in a directory that was not the worktree;
- `git worktree remove` failed on `__pycache__/` and the model reached for `--force`.

## 2. What ships

One tool, `worktree`, advertised up front (a core tool, see
[deferred-tools.md](deferred-tools.md)) on the direct path and on the CLI bridge:

| Action | What it does |
|---|---|
| `create` | `git worktree add` a branch (`branch`, default `forge/<name>`; an existing branch is checked out) from `base` (default the current `HEAD`, detached or not) at `path` (default `<main checkout>/.forge/worktrees/<name>`), then switch the session into it |
| `enter` | switch into an existing worktree of the same repository, found by `path`, `branch` or directory name; it may live anywhere (a sibling dir, `/mnt/work/wt/<name>`) |
| `list` | every worktree with its branch, marking the main checkout and the current one |
| `exit` | switch back to where the session was before its first switch (else the daemon's own worktree, else the main checkout); `remove: true` also removes the worktree being left |
| `remove` | remove a worktree the session is not in, then delete its branch |

Merging stays plain git: `exit`, then `git merge <branch>` (or open a PR from inside the worktree
first). Removal refuses a worktree with uncommitted or untracked files and keeps a branch that is
not merged into the main checkout's `HEAD`; `force: true` discards both, `keep_branch: true` keeps
the branch. The main checkout, the worktree the session is in, and a worktree a `forge serve`
daemon created for the session are never removed by this tool.

The default location is ignored inside the main checkout (a `*` `.gitignore` in `.forge/worktrees/`, and the same for
`.forge/worktree-owners/`), so the nested checkouts never show up in its `git status`.

`FORGE_SYSTEM` carries one line telling the model to use this tool instead of `git worktree add`
plus `cd`.

## 3. What follows the session on a switch

| State | On a switch |
|---|---|
| Tool registry workspace (file tools, relative paths, the shell's default cwd, evidence capture) | rebound to the new root |
| `shell` built with a workspace (`shell.sandbox` / `shell.scoped_cargo_target`) | uses the live binding, so it is not refused for leaving its starting root |
| `working_directory` in the env preamble | per request, follows |
| Cached branch (turn context, statusline) | re-read; linked worktrees are read through their `gitdir:` pointer |
| Lattice index and watcher, project context, `AGENTS.md` | rebuilt for the new root |
| Commit-discipline tracker | reset (it cached the old repo top) |
| `session.cwd` in the store | updated, so `--resume` and a daemon restart reopen the session in the worktree, and `forge worktree reclaim` sees it as live |
| `session.host_state` (host fingerprint) | re-recorded at the new root, so a resume inside the worktree is not reported as a host move ([session-host-move](session-host-move.md)) |
| TUI statusline | follows the session's workspace binding |
| Checkpoint root (`/undo` snapshots) | **kept** at the starting root, so snapshots outlive a removed worktree |
| Runtime "always allow" rules | persisted to the home checkout's `.forge/config.toml`, not the worktree's |
| Loaded config | kept; a worktree's own `.forge/config.toml` is not re-read |
| LSP | unchanged: servers are already keyed by each file's own project root |
| `/duel` merge-back | merges into the root the duel's worktrees were branched from |

## 4. Permissions and confinement

`list`, `enter` and plain `exit` are `ReadOnly` for the permission broker; `create`, `remove` and
`exit` with `remove: true` are `Shell`. With `[tools] confine_to_workspace = true` a session may
only move into roots under its starting workspace or an `extra_roots` entry; anything else is
refused with a message naming `extra_roots`. Without confinement, entering a worktree outside the
project is allowed like any other path the file tools may reach.

`shell.sandbox` (Landlock) keeps working in a worktree: the main checkout's `.git` is writable
(existing behaviour), and the null/zero/random/tty devices are now writable too — without
`/dev/null` every `git` command failed at startup under the sandbox
([shell-sandbox.md](shell-sandbox.md)).

## 5. CLI bridge and daemon sessions

`forge mcp-serve` runs the same `worktree_ops`, then rebinds its own registry and cwd so the rest
of the bridge turn works at the new root, and writes the root to the parent session's `cwd` in the
store. The parent follows it as soon as the bridge yields (`Session::adopt_bridge_workspace`), and
the next bridge spawn starts there. A parked persistent claude process is reused only for the
workspace it was spawned in ([persistent-bridge-transport.md](persistent-bridge-transport.md)).

A daemon session created with `worktree: true` owns that worktree through `session.worktree_path`:
the tool refuses to `exit` or `remove` it (the daemon's archive/merge flow owns its lifecycle), and
a second worktree the session enters exits back to it.

## 6. Verified live

`inferhub::glm-5.3-flash`, scratch repo with a local bare remote, `forge run --mode bypass`:

1. From the main checkout: `create` → `multi_edit`/`read_file` with relative paths → `pytest` and
   `git commit` through a cwd-less shell, all in the worktree → `exit` → `git merge` in main →
   `remove` (worktree and merged branch deleted). The main checkout's status stayed clean.
2. `enter` an existing worktree under an `extra_roots` directory outside the repo, write and
   commit there, `exit`; `enter` on a worktree outside every root was refused with
   `confine_to_workspace` on.
3. Detached-HEAD main checkout with `shell.sandbox = true`: `create`, commit, `exit` with
   `remove: true, keep_branch: true` (first refused on `__pycache__/`, which the model then
   deleted), `enter` a sibling worktree, commit, `exit`.
4. `forge mcp-serve` driven over stdio: `create`, relative `write_file`, cwd-less commit, `exit`;
   the store's `cwd` tracked each move, and `forge run --resume` launched from the main checkout
   reopened the session in the worktree. With the session's `worktree_path` set, `exit` was refused.

## 7. Not built

No `/worktree` TUI command: the user can ask the session, and the statusline already shows where
it works. No merge action: `git merge` and `open_pr` already cover it, and the branch-deletion check
catches an unmerged branch.
