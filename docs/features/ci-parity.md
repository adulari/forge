# Feature: CI parity (Forge runs your CI before it pushes)

> **Status: shipped.** `forge ci plan|run|status`, `/ci`, the `/goal` finish gate and the pre-push
> check of ship and the merge queue. The gate is off by default; opt a project in with `[ci] gate = "auto"` in `.forge/ci.toml`.

## 1. Problem

Gates were opt-in (`/goal --gate "<cmd>"`) and nothing read `.github/workflows`. A check the model
never ran could not block completion, and PR Watch only reacted after remote CI had already failed.
Local checks also drifted from CI: `ci.yml` sets `RUSTFLAGS="-D warnings"` at workflow level, so a
plain local `cargo clippy` reported "clean" twice (#1138, #1140) while CI was red; the architecture
size guard failed every later PR twice; a Windows-only break reached a release. Each avoidable red
round costs 20-90 minutes on a one-runner CI.

## 2. What it does

Forge derives a **CI plan** from `.github/workflows/*.y{,a}ml`:

- Workflows that run on `push` / `pull_request` (a `paths:` filter is honoured against the files
  changed since `origin/main`; a workflow with `schedule` / `workflow_dispatch` only is skipped).
- Jobs on a Linux runner. Windows and macOS jobs, `container:`, `services:`, `environment:` and
  reusable-workflow jobs are skipped.
- Their `run:` steps, in file order, with workflow, job and step `env:` applied (so
  `RUSTFLAGS=-D warnings` reaches clippy), `working-directory`, `shell` and `timeout-minutes`.
  `strategy.matrix` lists of plain values are expanded (at most 16 combinations; `include`,
  `exclude` and computed matrices skip the job).
- `uses:` steps are skipped, except well-known setup actions (`actions/checkout`,
  `actions/setup-*`, `dtolnay/rust-toolchain`, `Swatinem/rust-cache`, ...) which are no-ops
  locally; the plan counts them.

Every skip is listed with its reason, never dropped: needs a secret / repository variable / GitHub
context (`${{ secrets.X }}`, `${{ github.event_name }}`, `${{ steps.x.outputs.y }}`), a network or
deploy command (`npm publish`, `gh release`, `git push`, ...), a `git fetch`/`git pull` that
changes the clone's history depth (`--depth`, `--shallow-since`, `--unshallow`, ...; the step would
run in your own clone and make it shallow), runner housekeeping (a script that
writes `$GITHUB_ENV` / `$GITHUB_OUTPUT`), a cleanup step (`if: always()`), another `if:` condition,
a non-bash shell. `${{ runner.temp }}`, `${{ runner.os }}`, `${{ github.workspace }}`,
`${{ matrix.* }}` and `${{ env.* }}` are resolved.

A workflow the reader cannot parse is skipped as `unparsed: line N: ...` rather than guessed at.
The reader (`forge-config::ci_parity::yaml`) is a strict YAML subset with no dependency: maps,
sequences, plain / quoted scalars, `|` and `>` block scalars with chomping, flow collections,
comments, `${{ }}` kept verbatim. Anchors, aliases, tags, merge keys, multiple documents, tab
indentation, duplicate keys and explicit block indentation are errors.

## 3. Using it

| Command | What it does |
|---|---|
| `forge ci plan [--json]` | Print the plan: steps with env, then every skip with its reason. |
| `forge ci run [--keep-going] [--fresh] [--step <text>]` | Run it. Fail-fast by default; a step that passed on this exact tree (HEAD + uncommitted state) last time is not run again unless `--fresh`. |
| `forge ci status` | The last run in this worktree. |
| `/ci` (TUI) | The same plan as a list: pass / fail / skip with the reason, last run, `r` to run, Enter to open the focused row (a step's command, env and output tail, a skip's reason). A failed run lands on the failing step with its output open. |
| Local CI (desktop, web, phone) | Session menu or the command palette: the plan with each step's job, command and environment, the skips with their reasons, the last result with the output tail of what failed, and a Run button. |

The phone and desktop read `GET /api/sessions/{id}/ci` and run `POST /api/sessions/{id}/ci/run`. Both
are bridged for Anywhere (`session_ci`, `session_ci_run`) and need no ship opt-in: the run is a
session action under the same `prompt` scope as a message, since it executes the repository's own
workflow commands the way a turn does. A view-only device sees the snapshot summary below and can
read the plan, but cannot run it.

### On the session screen

The remote snapshot carries an additive `ci` object once the repository has had a run in the
daemon (or has a saved one): `runnable` / `skipped` counts, the `running` step while a run is going,
`steps` with `pass` / `fail` / `skip` / `not_run` per step, `finished_unix`, `ok`, and `blocked`,
the step that is holding a goal (`via: "goal"`) or a push (`via: "push"`, ship and the merge queue)
back. The session screen shows `blocked` as a banner with an Open button, and a run in flight as a
progress line. A later passing run clears the block.

A step whose command is not installed here (exit 127, a missing cargo subcommand) is reported as
`skipped (not installed)`, not as a failure.

## 4. Where the plan is enforced

- **`/goal` finish gate.** When a goal (or a contract goal) reaches completion, the plan runs as a
  quality gate named `forge ci run`, in addition to any `--gate`. A failing step goes back to the
  session as the next prompt: step name, key, command, environment and the output tail. Normal
  gate retries (3) apply; exhaustion stops the goal, never reports success.
- **Ship.** `forge ship` / `/ship` runs the plan after the commit and before the push; a failure
  refuses the `push` step with the same text.
- **Merge queue.** After merging the base in and running the project's `verify` actions, the plan
  runs before the queue pushes; a failure hands the failing step to the session.
- **PR Watch fix turns.** The prompt asks the session to run `forge ci run` before pushing, but the
  session pushes on its own, so Forge cannot intercept that push. What the watch does instead:
  once a fix turn has left a new commit and the session is idle, it runs the plan on that head
  (`serve_pr_watch/local_ci.rs`), holds the poll and the merge while it runs, and sends a failing
  step back to the session as the next prompt instead of waiting for the remote CI to fail the
  same way. Two failed attempts in a row block the watch with the reason. A passing plan, a repo
  without runnable steps, a project that has not set `[ci] gate = "auto"` and `FORGE_CI_GATE=off`
  leave the watch as it was.
  The check state is in memory: a daemon restart drops it and the watch polls as before.

## 5. Configuration (`.forge/ci.toml`, or a `[ci]` table in `.forge/config.toml`)

```toml
[ci]
gate = "off"                  # default. "auto": finish gate and pre-push check (opt in per project)
step_timeout_minutes = 30     # a step's own timeout-minutes wins
total_budget_minutes = 120    # remaining steps are reported not-run when it is spent
exclude = ["ci.yml::test::*"] # `*` globs over workflow::job::step name
include = ["Enforce standing docs freshness"] # pull a skipped step back in
```

`FORGE_CI_GATE=off` in the environment disables every automatic use at once. The step results are
stored in the worktree's git directory (`forge-ci-last.json`), never in the tree.

## 6. Known limits

- The plan trusts the repository's workflow files the way a CI runner does: running it executes
  their `run:` commands. Automatic use happens only inside sessions the owner started.
- Steps depending on earlier `uses:` steps that were skipped (a tool installer, artifacts) fail or
  report "not installed"; exclude them in `.forge/ci.toml`.
- Path filters need `origin/main` or `origin/master`; without a base, every workflow is planned.
- No evaluation of `needs:` ordering: steps run in file order.
