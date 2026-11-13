# Host pressure guard

A dev box that silently fills up shows itself as a confusing build failure hours later: 33
per-worktree `target/` dirs once held 361 GB and CI failed with no log; local builds starved memory
until earlyoom killed them (exit 143). `forge serve` now watches its own host and says so first.

## What it does

Every `[pressure] sample_secs` (60) the daemon reads the free space of the filesystems it works in
(its data dir, the default project, each project root, every live session's worktree; one reading
per distinct filesystem) and the available memory (`sysinfo`, i.e. `/proc/meminfo` on Linux).

| Level | Disk | Effect |
|---|---|---|
| warn | free < `disk_warn_free_gb` (20) or < `disk_warn_free_percent` (10) | status line, one push, warning on new worktree sessions |
| floor | free < `disk_floor_free_gb` (3) or < `disk_floor_free_percent` (2) | the above, and new worktree sessions are refused (HTTP 507, code `host_disk_floor`) |
| memory | available < `memory_low_mb` (1024) or < `memory_low_percent` (5), for 3 samples in a row | warn only; never refuses; informational, no Reclaim, its push never mentions reclaiming |

A level rises as soon as a reading crosses its threshold and falls only once the reading is 25%
past it, so a disk hovering at the line does not flap the push or the chip. A threshold of `0`
turns that check off; `[pressure] enabled = false` turns everything off. Only worsening
transitions push (`ok` to `warn`, `warn` to `floor`); recovery is silent.

## Surfaces

- Daemon: `pressure` in `GET /api/identity` (with a `cause` of `disk` or `memory`; additive; bridged over Forge Anywhere) and a
  `host_pressure` check in `GET /api/diagnostics`. `forge doctor` prints a "Host pressure" line.
- Push: one `notify`-kind notification per transition through the existing Web Push / APNs /
  Anywhere / desktop channels.
- Creating a worktree session (`POST /api/sessions` with `worktree` or `branch`): under warn the
  session is created and the answer carries `pressure` with the reclaimable bytes; under the floor
  nothing is created and the refusal body carries the same block.
- Reclaim: `GET /api/host/pressure?cwd=` previews, `POST /api/host/reclaim {cwd}` runs the same
  classifier as `forge worktree reclaim --artifacts --yes` (merged, clean, pushed, unused
  worktrees and stale `target/` dirs; nothing else). `cwd` must be a workspace the daemon already
  works in; without one, every repo the daemon knows (default project, project roots, live
  sessions) is previewed or reclaimed and the outcome summed. Not bridged over Anywhere (it deletes files): a relayed phone is told the command to
  run on the host.
- TUI board: a header chip, `P` (or a click on the chip) to reclaim after a confirmation, and a
  Reclaim dialog when a new worktree session is refused.
- Desktop, web and phone: a banner above every screen (Reclaim, after a confirmation that shows what can be freed, when connected directly and a disk is the cause), a
  command-palette entry, and a Reclaim button under the refusal on the new-session screen.

## Not covered

Worktrees created by a local `forge run --worktree` (no daemon) are not gated; `forge doctor`
and `forge worktree list` report the same numbers there.
