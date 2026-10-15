# `forge bench recall`: what a session still knows after compaction and resume

Compaction folds old turns into a summary, and a resume rebuilds the session from the store. Both
can lose things the user said an hour ago. `forge bench recall` measures how much survives, with
deterministic scoring and no judge model.

```
forge bench recall [--model provider::model] [--turns-scale N] [--skip-resume]
                   [--keep-workdir] [--json] [--price-in USD --price-out USD] [--mock]
```

The default model is `inferhub::glm-5.3-flash`. `--mock` uses the offline provider: it exercises
the plumbing in seconds, but a mock answers nothing, so its recall is meaningless.

## What it does

1. **Isolates itself.** A temp workspace (a small git repo), a temp store (`FORGE_DB`), temp
   config, data, state and cache dirs, and a generated config that turns off fleet publishing,
   self-MCP, the code index and model discovery. From the real config it borrows only the
   `[[providers.custom]]` entry (and window and price) of the pinned model, and a copy of the
   encrypted key store so the provider's API key resolves. Nothing is written to the real store,
   config or fleet, and the environment is restored on exit.
2. **Builds a real session** with the same constructor `forge run` and the TUI use (direct API
   path, Bypass mode, the pinned model) and plays about 23 scripted user turns. The early turns
   plant facts; the middle turns do cheap real tool work (reads, small edits, `wc`, `grep`) over a
   fixture that includes a few hundred lines of bulk data, so the context actually grows.
3. **Compacts twice**, by calling `Session::compact(false)`, which is exactly what `/compact`
   runs: once mid-scenario and once near the end. Facts planted between the two compactions
   ("late" facts) test a summary of a summary.
4. **Probes.** One question per planted fact, asked as an ordinary user turn that forbids tools
   ("answer from our conversation so far"), plus two behavior probes that ask for work and inspect
   what the model did: a function added to `src/parser.rs` must contain no `.unwrap()`, and a
   commit must start with the prefix the user demanded.
5. **Resumes.** The session is dropped and rebuilt from the store (`Session::resume`), and the
   same probes run again.

Each probe is independent: the workspace is snapshotted first, then the turn is undone (`/undo`)
and the workspace restored afterwards, so neither a question, its answer nor a stray edit is
visible to the next probe or to the resume.

## Fact categories

| Category | Planted as | Probe checks |
|---|---|---|
| `constraint` | "never use unwrap in src/parser.rs", "commit messages start with JIRA-123", a late rustdoc rule | the file / prefix / section |
| `correction` | "read config.toml" then "no, the config file is settings.toml" | the corrected file name |
| `decision` | a choice plus its reason (hand-rolled parser over `nom`, `u32` seconds for timeouts, ASCII-only lexer) | the choice and a reason keyword |
| `location` | a function added to a named file, a helper renamed in a named file | both names |
| `error_fix` | an error code printed by a script, the constant value that fixed it | the code, the value |
| `pending` | two TODOs the user said not to do yet | what they are |
| `number` | a port, a latency target, a benchmark budget | the number |
| `behavior` | the unwrap rule and the commit prefix | the file or commit the model produced |

Scoring is a regex match (case-insensitive, every `all` pattern must match and no `none` pattern
may). A memory question that used tools anyway (for example by re-reading a file) is counted and
flagged in the report, because its score may be inflated.

## Reading the output

The table has one row per category with `passed/total pct` after compaction and after resume, a
total, early-versus-late rows, the message counts of each compaction, and the token and cost
totals. `--json` prints the same as one JSON object on stdout (per-probe results included, with the
text the matcher saw); progress always goes to stderr. `--keep-workdir` keeps the temp workspace
and prints its path.

`--turns-scale N` multiplies the number of filler tool-work turns (below 1 it truncates, above 1 it
cycles extra reads); the planted turns never change. `--skip-resume` stops after the first probe
phase.

Cost is whatever the store recorded for the run. Models the catalog cannot price record zero, so
pass `--price-in` and `--price-out` (USD per million tokens) to have the bench price them.

## Adding a fact

The scenario is one static table in `crates/forge-cli/src/bench/recall/scenario.rs`. A fact is a
`Probe` on the turn that states it: an id, a category, the question, a `Matcher`, and a known-good
and known-bad sample. The unit tests run every probe's samples through the scorer, so a matcher that
accepts the wrong answer or rejects the right one fails CI.

Keep planted values out of the fixture (`fixture.rs`): the fixture test fails if a planted string
appears in a file the model can read.

## Baselines

[recall-baseline-2026-10](../benchmarks/recall-baseline-2026-10.md) holds the first run, on the
behavior before the compaction rework. Headline numbers belong to the recorded model, commit and
date.

## Code

- `crates/forge-cli/src/bench/recall.rs`: options and the run.
- `crates/forge-cli/src/bench/recall/scenario.rs`: the data.
- `crates/forge-cli/src/bench/recall/driver.rs`: the turn/compaction/probe loop over a `Subject`
  trait (a scripted fake in the tests, a real `Session` live).
- `crates/forge-cli/src/bench/recall/live.rs`: the isolated environment and the live subject.
- `crates/forge-cli/src/bench/recall/score.rs`, `render.rs`: matching, roll-ups, the table.
- `crates/forge-cli/src/bench/recall/fixture.rs`: the throwaway repo.
