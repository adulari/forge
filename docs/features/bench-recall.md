# `forge bench recall`: what a session still knows after compaction and resume

Compaction folds old turns into a summary, and a resume rebuilds the session from the store. Both
can lose things the user said an hour ago. `forge bench recall` measures how much survives, with
deterministic scoring and no judge model.

```
forge bench recall [--scenario standard|hard] [--model provider::model] [--compact-cap TOKENS]
                   [--turns-scale N] [--skip-resume] [--keep-workdir] [--json]
                   [--price-in USD --price-out USD] [--mock]
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
4. **Checkpoints** the store and the workspace as the scenario left them, then probes in two
   phases. One question per planted fact, asked as an ordinary user turn that forbids tools
   ("answer from our conversation so far"), plus work probes that inspect what the model did: a
   function added to `src/parser.rs` must contain no `.unwrap()`, and a commit must start with the
   prefix the user demanded.
   - **after-compaction** runs the probes in sequence on the live session. There is no way to
     rewind it between probes: `/undo` after a compaction reloads the whole uncompacted history
     (observed on 2026-10-16, see the baseline report), so an undone probe would quietly hand the
     next one the full transcript. An earlier answer can therefore leak into a later probe, and a
     compaction can fire while probing; both are reported.
   - **after-resume** gives every probe its own session, resumed from the checkpoint with
     `Session::resume` and discarded afterwards (the store files are restored from the checkpoint
     before each one). This phase is isolated and is the cleaner number.
   The workspace goes back to the checkpoint after every probe in both phases.

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
| `tool_result` | (hard) a value only ever visible in a file the model read or in command output | the value |
| `work_state` | (hard) "which files have you changed and what is left?" | every file in the real git history since the baseline commit is named; every pending item is named |
| `task_list` | (hard) three `update_tasks` tasks, two later marked done | the open one is named, the done ones are not |

Scoring is a regex match (case-insensitive, every `all` pattern must match and no `none` pattern
may). A memory question that used tools anyway (for example by re-reading a file) is counted and
flagged in the report, because its score may be inflated.

## The hard scenario (`--scenario hard`)

The standard scenario sits near the ceiling: a capable model keeps 94% of it. The hard one is built
to look like the real complaint, forgetting in a long session, worst after compaction:

- **Auto-compaction under pressure.** The temp config carries `[compact_cap]` of 24,000 tokens
  for the pinned model (`--compact-cap` changes it), so the real auto-compact, floor-prune and
  window-trim path fires repeatedly, not only the two manual `/compact`s. The report counts auto
  against manual compactions and names the model that wrote each summary (read from the session's
  "compacted N -> M (summary via model)" note); a compaction that changed nothing, or that fired
  while probing, is counted separately.
- **About 40 turns with large tool outputs**: whole-file `read_file` reads of generated files
  (a 400-row table, a 700-line log, notes, tables), a 220-line command output, so pruning cuts
  results before they are summarized.
- **Facts at varied depths**: early, mid-session and late, plus values that exist only in a tool
  result (a quota on row 233 of the table, a checksum on line 137 of a command's output).
- **An `AGENTS.md` rule** (every new function needs a doc comment starting `Forge:`), read at
  session start and probed by a work task after the compactions, and **a mid-session
  correction** (4 spaces, later "use tabs"), probed by a work task that checks the file for tabs.
- **Work state**: the reply to "which files have you changed and what is left?" is scored against
  the real `git` history (every changed file named, every planted pending item named).
- **A task list**: `update_tasks` with three tasks, two later marked done; the probe asks which
  are still open.

## Reading the output

The table has one row per category with `passed/total pct` after compaction and after resume, a
total, early-versus-late rows, the message counts of each compaction, and the token and cost
totals. `--json` prints the same as one JSON object on stdout (per-probe results included, with the
text the matcher saw); progress always goes to stderr. `--keep-workdir` keeps the temp workspace,
store and config and prints their paths.

`--turns-scale N` multiplies the number of filler tool-work turns (below 1 it truncates, above 1 it
cycles extra reads); turns that state a fact never change. `--skip-resume` stops after the live
phase.

Cost is whatever the store recorded for the run. Models the catalog cannot price record zero, so
pass `--price-in` and `--price-out` (USD per million tokens) to have the bench price them.

## Adding a fact

The scenario is one static table in `crates/forge-cli/src/bench/recall/scenario.rs`. A fact is a
`Probe` on the turn that states it: an id, a category, the question, a `Matcher`, and a known-good
and known-bad sample. The unit tests run every probe's samples through the scorer, so a matcher that
accepts the wrong answer or rejects the right one fails CI.

Keep planted values out of the fixture (`fixture.rs`): the fixture test fails if a planted string
appears in a file the model can read. Values that are meant to be tool-result-only (the hard
scenario's quota and checksum) are tested to appear exactly once.

## Baselines

[recall-baseline-2026-10](../benchmarks/recall-baseline-2026-10.md) holds the runs on the
behavior before the compaction rework (standard and hard). Headline numbers belong to the recorded model, commit and
date.

## Code

- `crates/forge-cli/src/bench/recall.rs`: options and the run.
- `crates/forge-cli/src/bench/recall/scenario.rs`, `scenario/hard.rs`: the data.
- `crates/forge-cli/src/bench/recall/driver.rs`: the turn/compaction/probe loop over a `Subject`
  trait (a scripted fake in the tests, a real `Session` live).
- `crates/forge-cli/src/bench/recall/live.rs`: the isolated environment and the live subject.
- `crates/forge-cli/src/bench/recall/score.rs`, `render.rs`: matching, roll-ups, the table.
- `crates/forge-cli/src/bench/recall/fixture.rs`: the throwaway repo.
