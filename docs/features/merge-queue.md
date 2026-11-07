# Merge queue

The PRs Forge watches in `merge` mode land one at a time, each tested against the main the
previous one produced. A backlog drain or a night of shipped sessions no longer ends with a
pile of PRs that all conflict the moment the first one merges.

## What it does

PRs of one remote repository and base branch (`origin URL @ base`; checkouts and worktrees of the
same remote share a queue) form a queue in the order their watches were armed. Only the head may
merge. Every other PR keeps PR Watch's fix turns for failing checks and comments but is held at
the merge step with its place in line (`#2 in the merge queue, behind PR #7`).

For the head, before PR Watch looks at it, the daemon:

1. **Updates it.** `git fetch`, then `git merge origin/<base>` in the session's own worktree, committed
   by the worktree's configured git identity. It never uses `gh pr update-branch`: those commits are
   authored by github-actions[bot], which parks CI at `action_required` and deadlocks the PR.
2. **Resolves changelog-only conflicts.** When every conflicted path is a `CHANGELOG.md` (any directory,
   so a byte-identical package copy resolves identically), and both sides only added lines (nothing
   existing was edited or removed), the entries of both sides are kept, the branch's first and the
   base's after. A strict check refuses the result if any conflict marker is left. Any other
   conflict, or a changelog change that is not a pure addition, aborts the merge and hands the PR
   to its session as a fix turn that names the paths. Nothing is resolved silently or partially.
3. **Verifies the merged result.** The project's `verify` and `check` actions (`[[actions]]` in
   `.forge/config.toml`, trusted projects only; see [project-actions.md](project-actions.md)) run in
   the worktree, in declared order, each capped at 15 minutes. With none configured the step is
   skipped and the update note says so. A failure keeps the merge commit local, pushes nothing, and
   sends the output tail to the session.
4. **Pushes** with a plain `git push` (never forced), then waits at least 45 seconds and for GitHub
   to report the pushed commit, so a PR whose checks have not registered yet is not mistaken for a
   green one. PR Watch then waits for the required checks and merges as before.

The next PR starts only after the previous one merged (or was handed back), so it is updated
against the new main.

## Not blocking the queue

A PR that conflicts for real, fails its verification, sits on a detached HEAD or has its push
refused is marked `skipped` with the reason and the queue moves on. When its session pushes (the
PR head changes), it re-enters at its original place. A dirty worktree, a busy session or a failed
fetch only wait; they are retried. A PR whose watch blocks (closed, giving up after fix turns)
leaves the queue like any finished one.

## Surfaces

- `forge prs queue [--json]`: position, state and reason per PR, per repository.
- `GET /api/pr/queue`: `{queues: [{repo, rows: [{session_id, pr_number, pr_url, position, state,
  reason, pushes, last_update}]}]}`; state is `head`, `queued`, `checks`, `skipped`, `merged` or
  `blocked`. `GET /api/pr/watches` carries the same `queue` object on each merge-mode watch.
- Phone and desktop: the PR Watch panel and the Schedules list label a queued watch (Queued to
  merge, Landing next, Updated in checks, Handed back) with the position or reason.
- Away brief (`forge brief`, `B` on the board): a `Merge queue: 3 waiting, next PR #12 (checks),
  1 handed back` line.

Not built: a dedicated interactive queue overlay in the TUI (the board shows the brief line and
the CLI prints the table), and reordering the queue by hand.

## Tests

`crates/forge-cli/src/serve_merge_queue/tests.rs` runs the whole flow against a local bare
remote and a fake `gh` that answers from it (no network, no GitHub): three PRs that each add a
CHANGELOG entry under one heading plus distinct code, a fourth with a real code conflict, serial
merges in order with verification seeing the previous PRs' code, the conflicting PR handed back
without blocking the rest and landing after its session resolves it. The drill is
`~/.cache/forge-burn-demo/run4/mergeq/drill.sh`.
