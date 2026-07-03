# `forge queue` — the overnight autopilot

> **Status: shipped (core).** Queue big tasks during the day; a drain runs each one headless in
> its own isolated git worktree — budget-capped, optionally assay-gated — and leaves a
> review-ready `autopilot/<slug>` branch plus a morning digest. No daemon: the drain is a plain
> foreground command, schedulable with `forge schedule`.

## What it does

```bash
forge queue add "migrate the auth module to the new API" --budget 2.50
forge queue add "add integration tests for billing" --mode bypass
forge queue                          # list: id, status, budget, task
forge queue run [--gate high] [--max N]
forge queue report                   # the morning digest
forge queue remove <id-prefix>
```

Overnight setup — one timer that drains whatever is queued:

```bash
forge schedule add "run the queue" --at 23:00    # or wire a cron/systemd timer at `forge queue run`
```

## Task lifecycle

`pending → running → done | empty | gated | over-budget | failed`

| Status | Meaning |
|--------|---------|
| `done` | run finished and produced a result branch |
| `empty` | run finished clean but changed nothing (no branch) |
| `gated` | `--gate <sev>` assay pass found findings at/above the severity; branch still kept |
| `over-budget` | killed when session cost crossed `--budget`; partial work kept on the branch |
| `failed` | the run errored and nothing was committed |

## How a drain works

For each pending task (this project's cwd only, oldest first, sequential on purpose):

1. **Claim** — a single-shot `pending → running` UPDATE, so concurrent drains never double-run.
2. **Worktree** — `WorktreeGuard::create` (same isolation as subagents/duel): branch from HEAD,
   shared cargo target, auto-removed on drop.
3. **Run** — spawn the forge binary itself: `forge run "<task>" --output-format stream-json
   --mode accept-edits` (mode/model overridable per task) with cwd = the worktree. The NDJSON
   stream is folded live: `init` → session id, `usage` → cumulative cost, `result` → summary.
4. **Budget** — if `--budget` was set and the streamed cost crosses it, the child is killed;
   whatever it wrote so far still gets committed and the task is marked `over-budget`.
5. **Gate** (opt-in `--gate low|medium|high`) — `forge assay run --scope diff --fail-on <sev>`
   inside the worktree, before committing (assay's diff scope reads the working tree). Exit 2
   marks the task `gated`; the branch is kept either way — the gate labels, it doesn't destroy.
6. **Branch** — `commit_worktree` snapshots the edits (excluding `.cargo` shim +
   `.forge/checkpoints` session plumbing), then `git branch -f autopilot/<slug>-<id6>` pins the
   result before the guard's drop deletes its own temporary branch.
7. **Record** — one `finish_queue_task` write: status, session id, branch, summary, cost, gate.

The drain ends with a desktop notification (notify-send / osascript / msg.exe, best-effort) and
points at `forge queue report`.

## Storage

`queue_task` table (migration 0005; also in the base schema). Machine-local like `schedule` —
deliberately **not** in `PORTABLE_METADATA_TABLES` (cwd + branches don't travel with
`forge migrate`). A running task refuses `remove`.

## Verified

- Store roundtrip test (add / cwd-filter / prefix / single-shot claim / finish / remove guards).
- Pure-helper tests: NDJSON fold, slugify, char-boundary truncate.
- Live e2e (mock provider, scratch repo, isolated DB): two tasks drained → two `autopilot/*`
  branches each containing exactly the written file (the `.forge/checkpoints` leak was found and
  fixed here — see `commit_worktree`), zero leftover worktrees, digest lists branches + replay
  pointers.

## Deferred (next PRs)

- `/queue` in the TUI + a digest card on the next `forge chat` after a drain.
- `forge queue schedule` sugar (installs the OS timer directly instead of via `forge schedule`).
- Parallel drains (needs per-provider fan-out caps at the mesh layer to be safe).
