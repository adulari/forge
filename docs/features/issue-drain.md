# Backlog drain

Run several GitHub issues as one batch: at most N sessions at once, an optional shared spend
envelope, and one line in the brief.

- Each issue starts exactly like a single start (`forge issues --start`): fresh-base worktree, the
  issue quoted as untrusted data, PR Watch armed without raising the permission mode.
- A slot frees only when the session opens a PR, goes idle, fails, its watch blocks, or the session
  ends. A session stopped on a permission prompt or question keeps its slot, so `--parallel 2` means
  at most 2 open, unfinished issue sessions at a time in any permission mode. The next queued issue
  starts when a slot frees. Default parallelism is 2 (max 8).
- `--budget claude:30%w` (same syntax as `/goal --budget`) is baselined when the drain is created.
  Once the slice is spent nothing new starts; when the window resets the drain rebaselines on the
  new reading and carries on. A `$5` budget sums the drain's sessions' cost and never resets.
- State is `drains.json` in the data dir, restored on daemon start. Running sessions come back with
  the fleet; items whose session is gone are marked blocked.
- Cancelling stops new starts; sessions already running are left alone.

Daemon routes: `POST /api/issues/drain`, `GET /api/issues/drains[?id=]`,
`POST /api/issues/drain/cancel`. `GET /api/brief` carries `drain.line`
(`Drain: 7 started, 2 running, 4 merged, 2 waiting on you, 1 blocked, 3 queued`).

Surfaces: `forge issues --drain 12,15,19 --parallel 2 --budget claude:30%w`, the `/issues` overlay
(`m` marks, `D` drains), and the Issues view on desktop and phone (multi-select, Drain sheet).
