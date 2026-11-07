# Feature: CI parity (Forge runs your CI before it pushes)

> **Status: shipped.** `forge ci plan|run|status`, `/ci`, the `/goal` finish gate and the pre-push
> check of ship and the merge queue. Opt out per repository with `[ci] gate = "off"`.

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
deploy command (`npm publish`, `gh release`, `git push`, ...), runner housekeeping (a script that
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
| `/ci` (TUI) | The same plan as a list: pass / fail / skip with the reason, last run, `r` to run. |

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
- **PR Watch fix turns** only get a line in the prompt asking the session to run `forge ci run`
  before pushing; the session pushes on its own, so Forge cannot intercept that push.

## 5. Configuration (`.forge/ci.toml`, or a `[ci]` table in `.forge/config.toml`)

```toml
[ci]
gate = "auto"                 # "off": no finish gate, no pre-push check
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
