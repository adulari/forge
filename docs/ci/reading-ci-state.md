# Reading CI state without being misled

CI in this repo shows red things that gate nothing, and green PRs that cannot merge. Every trap
below was hit in real diagnosis, more than once, and each one produced a confidently wrong
conclusion before the evidence arrived.

## The one rule

**`statusCheckRollup` is authoritative. A raw check-runs listing is not.**

```sh
gh pr checks <n>
gh pr view <n> --json statusCheckRollup \
  --jq '[.statusCheckRollup[] | select(.name=="CI" or .name=="mobile checks" or .name=="security checks")]
        | map("\(.name)=\(.conclusion // .status)") | join("  ")'
```

Reach for `gh api repos/<owner>/<repo>/commits/<sha>/check-runs` only when you already know what
you are looking for inside a specific run. Do not decide whether a PR is blocked from it.

## Trap 1 — a red job is not a blocked PR

Only the aggregates gate merging: **`CI`**, **`mobile checks`**, **`security checks`**. Individual
jobs (`clippy`, `test (archlinux)`, `lint, typecheck, and tests`, …) are not required contexts.

A PR can carry a `completed/failure` job and still be perfectly green. Counting failing job names
is how a "four PRs are blocked" report gets written about PRs that were never blocked.

## Trap 2 — dispatched runs fail in ways that gate nothing

`scripts/ci/changed-groups.sh` classifies a `pull_request` by its diff, but sends every other event
straight to `enable_all` — see the comment at the top of that file. A `workflow_dispatch` carries no
PR context, so on a Rust-only branch it enables `mobile_app` and runs the mobile npm audit that the
real PR run correctly *skips*.

The result is a branch touching no mobile file showing:

- `lint, typecheck, and tests: completed/skipped` — from the `pull_request` run, and
- `lint, typecheck, and tests: completed/failure` — from a `workflow_dispatch` run,

with the aggregate `mobile checks` **green**. Before concluding anything from a failing job, check
which event its run came from:

```sh
gh api repos/<owner>/<repo>/actions/jobs/<job_id> --jq .run_id
gh api repos/<owner>/<repo>/actions/runs/<run_id> --jq .event
```

## Trap 3 — the rollup does not pick the most recent check run

Two check runs with the same name can coexist on one SHA, from different check suites. The rollup
does **not** simply take the later one.

Observed: a dispatch run's `mobile checks=failure` completed 80 minutes *after* the `pull_request`
run's `mobile checks=success`, and the rollup still reported **SUCCESS**. Predicting that the later
failure would "win" and block the PR was wrong.

Do not reason about which check run wins. Ask the rollup.

## Trap 4 — a cancelled job fails the gate exactly like a real failure

All three aggregates are `if: always()` over their dependencies and share this shape (the wording of
the message differs per workflow):

```yaml
case "$result" in
  success|skipped) ;;
  *) exit 1 ;;   # anything else — including `cancelled`
esac
```

So `cancelled` is indistinguishable from `failure` at the gate. This matters because heavy jobs get
cancelled a lot — all three workflows use ref-keyed `cancel-in-progress: true`, so any new run for a
branch kills the in-flight one mid-job.

Dependabot #924 sat **two weeks** looking broken: `test (archlinux)` and `clippy` had both passed
and only `release-build` was cancelled. Nothing was wrong with the dependency bump.

When an aggregate is red, check whether anything genuinely failed:

```sh
gh api repos/<owner>/<repo>/actions/runs/<run_id>/jobs \
  --jq '.jobs[] | "\(.conclusion)  \(.name)"'
```

All `success`/`skipped`/`cancelled` and no `failure` means churn, not breakage. Re-run it.

## Why a PR can sit queued for hours with nothing wrong

Job labels decide which runner may take a job, and they are not evenly served:

- `[self-hosted, linux, x64]` — any runner.
- `[self-hosted, linux, x64, heavy]` and anything wanting `release` — **one runner only**.

The `heavy` restriction is deliberate, and `ci.yml` states why: workspace-wide compiles are the
box's big memory consumers, so they are serialized onto one 12G-capped runner to stop concurrent
PRs OOMing the host — on a 30G machine, three 8G runners compiling at once repeatedly killed all
runner services.

The practical consequence is that the light jobs finish immediately and the remaining backlog is
entirely on the single machine, while the others sit idle. Measured on one ordinary afternoon:
**104 queued jobs needing that runner, 0 that the other two were permitted to take**, draining at
roughly one job every five minutes.

A merge to `main` makes every open PR out of date, and each one re-queues its heavy jobs — on the
order of 45 jobs for a normal open-PR set. So "my PR has been queued for hours" is usually the
expected behaviour of the queue, not a fault to investigate.

## Trap 5 — a job can outlive its own timeout, and then it never resolves

A required check that never concludes blocks a PR exactly like a failure, but reads as "still
running" in every view. It is worth knowing this can be a dead job rather than a slow one.

Measured 2026-08-11 on #1028. The `mobile checks` aggregate declares `timeout-minutes: 5`, and:

    job  93817487082   name=mobile checks   status=in_progress   started=14:42:30
                       runner_name=archlinux-2   completed_at=null

    ...still in_progress 23 minutes later, on a 5-minute timeout.

Two things make it identifiable:

- **The parent run had already finished.** `actions/runs/<id>` reported `completed/success` while
  one of its own jobs was still `in_progress`. A run cannot legitimately complete with a live job.
- **The named runner was idle.** `actions/runners` reported `archlinux-2  busy=false` while GitHub
  still believed a job was executing on it.

The timeout does not save you here: enforcement depends on the runner reporting back, so a runner
that goes away mid-job leaves the record hanging indefinitely. Waiting is not a fix — nothing is
coming.

**How to spot it,** across recent runs:

```sh
gh api repos/<owner>/<repo>/actions/runs/<run_id>/jobs \
  --jq '.jobs[] | select(.status != "completed") | "\(.name) started=\(.started_at) runner=\(.runner_name)"'
```

Any job listed there whose **run** is already `completed` is orphaned.

**The fix is a fresh context, not a re-run of the dead one.** Dispatch the workflow at the branch;
branch protection reads the most recent check run of that name on the SHA, so a new one supersedes
the stuck record:

```sh
gh workflow run mobile-typecheck.yml --ref <branch>
```

## Trap 6 — during a GitHub incident, REST and GraphQL disagree, and `gh` mostly speaks GraphQL

`gh pr view --json statusCheckRollup,mergeStateStatus` and `gh pr checks` read the **GraphQL** API.
When GitHub is degraded, that is frequently the degraded component, so the authoritative source in
[The one rule](#the-one-rule) can itself be stale.

On 2026-08-11 the status page reported *"Incident with GraphQL API Requests"* while REST
(`actions/runs`, `commits/<sha>/check-runs`, `pulls/<n>`) kept answering normally.

Before diagnosing anything strange — a PR blocked with no failing check, states that contradict
each other, jobs that will not settle — check whether the platform is having a bad day:

```sh
curl -s https://www.githubstatus.com/api/v2/summary.json | jq '.status.description, .incidents[].name'
```

Then re-read the same facts over REST and see whether the two agree:

```sh
gh api repos/<owner>/<repo>/pulls/<n> --jq '{mergeable, mergeable_state}'
gh api repos/<owner>/<repo>/commits/<sha>/check-runs --jq '.check_runs[] | "\(.name)=\(.status)/\(.conclusion)"'
```

This cuts both ways, so do not use an incident to explain away a real defect: when the two sources
were compared on #1028 above, REST **confirmed** the stuck `mobile checks` job rather than
contradicting it. An incident makes readings suspect, not wrong.

## Quick triage order

1. Is GitHub itself degraded? If the picture is incoherent, check the status page before anything
   else, and re-read the facts over REST (Trap 6).
2. `gh pr checks <n>` — is an **aggregate** actually failing? If not, the PR is fine; it is waiting.
3. If an aggregate is red, list its run's jobs. Any genuine `failure`, or only `cancelled`?
4. If a job did fail, confirm its run's `event` is `pull_request` before treating it as real.
5. If an aggregate is neither red nor green but never settles, check whether its job outlived its
   timeout while its run completed (Trap 5) — that one needs a dispatch, not patience.
6. Only then read the log.
