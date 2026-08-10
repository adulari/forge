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

## Quick triage order

1. `gh pr checks <n>` — is an **aggregate** actually failing? If not, the PR is fine; it is waiting.
2. If an aggregate is red, list its run's jobs. Any genuine `failure`, or only `cancelled`?
3. If a job did fail, confirm its run's `event` is `pull_request` before treating it as real.
4. Only then read the log.
