# Moved sessions: host fingerprint, notice, stale-path guard, `/reground`

A session can move to another machine: a store copy, an Anywhere history sync, or an Anywhere
handoff capsule. Before this feature it kept describing the first machine. Its history and its
compaction summaries named sibling worktrees, directory layouts and file contents from that
machine. When the username and home directory match, those absolute paths look plausible but
point at nothing. The model then got stuck, mixed up paths and invented files to fill the gaps,
because nothing on resume told it the machine had changed.

Code: `crates/forge-core/src/host_move/` (`fingerprint.rs`, `scan.rs`, `plan.rs`, `guard.rs`),
`crates/forge-store/src/host_state_store.rs`, migration #44.

## What is recorded

`session.host_state` (JSON, migration #44) holds:

- **fingerprint**: hostname, a hash prefix of `/etc/machine-id`, `$HOME`, the workspace cwd, the
  repo root, branch and HEAD, plus every git checkout around the workspace (the repo itself, its
  `git worktree` checkouts and the git checkouts in its parent directory), each with its remotes in
  canonical form (`host/owner/repo`, credentials stripped). Everything is read from `.git` files
  directly, with no `git` subprocess, so capturing it costs almost nothing.
- **stale**: previous-host paths that do not exist here, each with its local equivalent
  (`to`), how the match was made (`via`), and any other checkouts of the same repo.
- **local_repos**: the local checkouts known when the move was detected.

`Session::start` records the fingerprint. A handoff capsule carries it in
`HandoffSessionExport.host_state` (optional field, ignored by older destinations). The capsule now
also carries the compaction summary. Before, the destination resumed only the active tail and lost
everything the session had compacted.

## What fires when

| Moment | Effect |
| --- | --- |
| `Session::start` | records the fingerprint |
| The `worktree` tool switches the session (`enter`, `create`, `exit`, in-process or through `forge mcp-serve`) | re-records the fingerprint at the new root, keeping the stale map, so resuming inside a worktree is not a host move ([session-worktrees](session-worktrees.md)) |
| Resume (`forge chat --resume`, `forge run --resume`, daemon resume, a resumed handoff import) | compares the stored fingerprint with the current host |
| Machine differs (machine-id, or hostname when either side lacks one) or the cwd moved | appends **one** system notice and arms the guard |
| No fingerprint stored (a session from before this feature) and its history names **two or more** missing directories outside the workspace | same notice, which says the origin is unknown |
| Same host and same cwd | nothing; stale entries from an earlier move stay armed while their source is still missing |
| Recorded cwd missing on this host | the cwd is moved to the local checkout with the same git remote (or the same checkout name when no remote is recorded), keeping the subdirectory; a recorded worktree that does not exist here is cleared |
| `/reground` | always appends the notice, then runs a compaction whose summarizer is told to drop host-specific facts |

The notice is decided by a compare-and-swap on `host_state`
(`UPDATE … WHERE host_state IS <old>`). Once the new fingerprint is stored, later resumes match and
send nothing, and two processes resuming the same session cannot both announce the move. The notice
is persisted like any other system message and appended at the tail. That keeps it cache-friendly:
the prefix does not change, and it is sent once, not on every turn.

### The notice

It starts with `[host] This session moved: <reason>.` and then gives the before and after host and
workspace. It also lists the current layout: repo root, branch and HEAD, git worktrees, sibling
directories (git checkouts are marked) and top-level entries. Then comes the path map
(`previous → here (via)`), and finally the previous paths that have no local equivalent.

The wording is deliberately narrow. On a real moved session, a manual notice that said "never rely
on remembered paths … never claim a result you have not just seen" made the model conclude that the
whole session was broken and that its own tool calls were fake. The notice therefore says three
things:

- history from the previous host may name paths that are absent here
- tool outputs in this session are real and trustworthy
- check a remembered path once before using it, use the path map rather than recreating a missing
  file, and do not audit earlier tool calls

A test asserts that the reassurance is present and that blanket-distrust phrasing is absent.

### Path map

1. The previous repo root (or cwd) maps to the current one ("this session's workspace").
2. Each previous checkout that is missing here maps to the local checkout that shares a git remote
   ("same git remote …"). Preference goes to the same checkout name, then to the current workspace
   repo, then to the closest path. Other matches are listed as alternates.
3. Missing directories found in the history (for example `.../API/ss-ultimate-wt-x/src/main.rs`
   gives `.../API/ss-ultimate-wt-x`) map by checkout name: `ss-ultimate-wt-x` belongs to
   `ss-ultimate`. A missing leaf file never counts, and neither does anything under the workspace
   or `/.forge/`.

Local candidates are the checkouts around the workspace plus the parents of the missing roots. Only
when something is still unmapped does Forge scan `$HOME` for checkouts (depth 5, at most 4000
directories, skipping hidden directories, `node_modules`, `target` and similar).

## Stale-path guard

The guard runs inside `check_workspace`, so it covers every file tool and the shell's `cwd`. It
also forces concurrent read-only batches onto the serial path. It refuses a target only when that
target does not exist here **and** one of these holds:

- it lies under a stale previous-host root whose source does not exist here, or
- it is a new directory placed beside such a root (the same confusion, but invented rather than
  remembered).

The error names the re-rooted local path, says how it was matched, warns when the mapped file does
not exist either, and lists alternate checkouts. Creating a new file in a directory that exists is
never blocked.

## `/reground`

`/reground` is available in the TUI and in `forge serve` / remote, because both share
`dispatch_command`. It re-runs detection with `force`, which appends the notice even when the
fingerprint matches. It then hands off to the `/compact` background task with a one-shot directive.
The summarizer is told to drop absolute paths, layouts, file contents and outputs tied to the
previous host, and to keep goals, decisions, constraints, open tasks and conclusions in
repo-relative terms. The compaction re-summarizes the existing summary together with the older
active messages; the notice itself stays in the kept tail. On a transcript too short to compact,
only the notice lands. Like `/compact`, it is refused while a turn is running.

## Other host state audited

- **Lattice**: indexed per workspace root; `prune_stale_roots` drops roots that no longer exist at
  startup.
- **LSP, project config, evidence ledger, turn contract**: derived from the live workspace root
  on every session build or turn; nothing is persisted per host.
- **Checkpoints / undo snapshots**: stored under `<workspace>/.forge/checkpoints` and do not travel
  with the store, so `/undo` of a turn from before the move restores nothing rather than writing to
  old paths.
- **`worktree_path`**: cleared on resume when it does not exist on this host.
- **Auto-memory**: scoped by project path. After a cwd relocation, memories scoped to the old path
  are not loaded. They are lost, not wrong, and are left untouched.
- **Anywhere history sync**: a synced session row gets the receiving daemon's cwd and no
  fingerprint, so it relies on the history-based detection path above.
