# RFC: one merge queue across daemons

| Field | Value |
|-------|-------|
| Status | Implemented |
| Author | Forge |
| Created | 2026-11-20 |
| Builds on | docs/features/merge-queue.md |

## Problem

Each `forge serve` daemon has its own merge queue. With two daemons (a laptop and a build host)
watching PRs on one GitHub repository, both update, verify and land at the same time. Each lands
after merging main, so two in parallel race: one goes BEHIND or conflicts after the other lands,
and the CI spent on its verification is wasted.

## Decision

A per-repository lease held on GitHub itself, in a ref of that repository. No new server, no
Anywhere record kind, no service deploy. Only the lease holder updates, verifies and lands.

### Why a ref

GitHub's git refs API is already a compare-and-swap:

- `POST git/refs` fails with 422 when the ref exists, so exactly one of two daemons creates it.
- `PATCH git/refs/<ref>` with `force=false` fails with 422 unless the new commit descends from the
  current one, so exactly one of two daemons that read the same state moves it.

Rejected alternatives: a label or commit status on a PR (per PR, not per repository, and anyone
can edit a label, with no atomic take-over); a check run (needs a GitHub App); the Anywhere relay
or fleet sync (only works when both daemons share an account, and a new record kind needs a
service deploy).

### Shape

`refs/forge/merge-queue/<base>` (one per queue: `origin @ base`) points at a chain of commits, the first without a parent. Each commit uses the tree of `origin/<base>` (so no object has to be invented) and
carries the state in its message:

```
forge-merge-queue-lease v1 owner=<host>-<store hash> host=<host> pr=<n|-> renewed=<unix> released=<0|1>
```

`refs/forge/` is not under `refs/heads`, so it shows in no branch list and triggers no workflow.

Every state change is a new commit whose parent is the commit just read, written with
`force=false`:

| Event | Write |
|-------|-------|
| acquire (no ref) | create the ref |
| renew | new commit, `pr` and `renewed` updated, fast-forward |
| take over | same, when the holder is stale or released |
| release | same, `released=1` |

A 422 on a ref write means another daemon moved first: re-read and decide again (three tries).

### Timing

A holder renews at least every 60 s (the PR poll is 30 s; a renewal inside the window is answered
from memory with no GitHub call). A lease nobody renewed for 10 minutes is stale and any daemon
may take it over, so a dead or sleeping daemon never blocks the queue for long. Staleness is
judged by the reader's clock against the `renewed` stamp, so clock skew must stay well under
the margin (10 minutes against a 1-minute heartbeat).

### Who asks for the lease

Only a daemon's local queue head, the PR that would update or land next, asks for it, just before
the update step. It keeps it while it has any PR of that repository waiting in line (updating,
in checks, landing, or queued behind), then releases it once its sweep finds nothing left to land
there. A PR the owner holds, or one handed back, does not want the lease: holding the last waiting PR
gives the lease back. The owner's hold, release, front and retry act on the local queue exactly as
before, and a manual merge never needs the lease.

A head that does not get the lease stays queued, keeps PR Watch's fix turns for failing checks and
comments, and shows who is landing (`landing on ssu-dev: PR #123`).

### Degrading

If GitHub refuses any lease call (no `gh`, signed out, no push access, rate limit, an answer that
cannot be read), the daemon lands on its own for five minutes, with a warning on the queue
(`shared queue unavailable (...), landing on its own`), then asks again. This is exactly today's
behaviour, so the worst case of the feature is the status quo. A remote that is not on github.com
needs no lease and is left alone. Nothing waits for the lease without an expiry, so it cannot
deadlock.

A daemon that loses its lease while it works (it slept past the stale limit and another daemon took
over) sees the other owner on its next renewal and holds. The in-flight update step is not
interrupted: its push is a plain fast-forward push, so the worst case is one redundant update.

### Default

`[pr_watch] shared_queue = true`. On by default because the failure mode is the old behaviour and a
stuck lease costs at most ten minutes. The one visible effect is a hidden ref in the owner's own
repository; `shared_queue = false` removes the feature. It needs `queue = true`.

### Surfaces

The queue snapshot (`GET /api/pr/queue`, Anywhere `pr_queue`) carries a `lease` object per
repository: `{status: mine|other|unavailable, holder, pr, expires_at, text}`. The terminal `/queue`
overlay and `forge board` show it on the repository line, `forge prs queue` after the repository
name, and the Merge queue page (desktop and phone) as a badge with the line.

## Not done

- No live smoke against github.com (nothing may write refs there from a test). The GitHub calls
  are covered against a fake `gh` that implements the same create / fast-forward-only semantics on a
  bare repository; the degrade path covers a real answer that differs.
- A daemon that exits cleanly does not release; its lease goes stale.
- The lease serialises landing per queue, not fairness: a daemon with a long local line lands all
  of it before the other starts.
