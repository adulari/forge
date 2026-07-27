# Harness benchmark runners

## Forge vs. native Claude Code

`compare_claude_swe.py` runs matched SWE-bench arms through Forge's Claude CLI
bridge and native Claude Code. It supports only the live aliases advertised by
Claude's non-billing `initialize` response and requires both models to advertise
regular `high` effort.

```bash
PYTHONPATH=scripts/harness-bench \
python3 scripts/harness-bench/compare_claude_swe.py \
  --dataset /path/to/swe-verified.jsonl \
  --out /path/to/run \
  --worktree-root /path/to/reusable-clones \
  --forge-bin ./target/release/forge \
  --models 'opus[1m],sonnet' \
  --arms forge,raw-claude \
  --baseline-weekly-pct 21 \
  --max-weekly-increase-pct 9 \
  --observed-weekly-pct 21 \
  --max-new-trials 1
```

The runner:

- sets both harnesses to `high`, never `xhigh`;
- recreates the official base tree as one synthetic reachable commit with no
  remotes or upstream history;
- prepends the benchmark integrity rule and captures patches relative to the
  synthetic base, including agent commits;
- scopes Forge state to one cell so sessions and usage cannot leak across arms;
- records Claude CLI version, resolved model IDs, supported effort levels, Forge
  commit, and binary hash;
- uses additive Claude cache accounting and a separate 0.25 cache-read
  sensitivity measure;
- records one prediction file per arm/model for official Docker evaluation;
- fails closed before another provider call when weekly telemetry is absent;
- enforces exactly one new provider arm per invocation so Helm can be refreshed
  externally before every resume;
- accepts a forced external reading through `--observed-weekly-pct`;
- resumes only a manifest-compatible prefix with `--resume`.

Run the official evaluator with `swebench==4.1.0`; a process exit, non-empty
patch, or model assertion is not a resolve. When a study spans multiple
quota-gated run roots, combine them with:

```bash
PYTHONPATH=scripts/harness-bench \
python3 scripts/harness-bench/analyze_claude_swe.py \
  --run-dir /path/to/sonnet-hard-run \
  --run-dir /path/to/opus-hard-run \
  --run-dir /path/to/easy-medium-run \
  --out-json /path/to/official-analysis.json \
  --out-markdown /path/to/official-analysis.md
```

The analyzer binds only explicit `resolved_ids`, `unresolved_ids`, and
`empty_patch_ids`. SWE-bench can list unevaluated prediction rows under
`submitted_ids`, so that field must never be treated as an outcome. Missing or
duplicate outcomes, evaluator errors, incomplete pairs, and missing wall/token
telemetry fail the analysis.

## Forge OAuth vs. raw Codex

This runner performs paired, isolated coding trials through:

1. Forge's direct `codex-oauth::<model>` provider and Forge tool loop.
2. The authenticated raw `codex exec` CLI with the same bare model.

It is intentionally narrow: the current study covers GPT-5.6 Sol, Terra, and Luna at `xhigh`
reasoning effort. Each arm receives the exact same fixture and user prompt in a new Git repository.
Correctness is decided by independent deterministic commands after the agent process exits—not by
the model's final claim.

The source fixtures and acceptance commands are the current versions under `scripts/manual-e2e/`.
Raw provider events, stderr, patches, verification logs, manifests, and per-trial summaries are
retained under an external output directory. No OAuth token or auth file is copied.

Example:

```bash
scripts/harness-bench/compare_codex_oauth.py \
  --out "$XDG_DATA_HOME/forge/harness-bench-20260726/baseline" \
  --forge-bin ./target/debug/forge \
  --baseline-weekly-pct 0
```

Important accounting rules:

- Current Forge Responses telemetry and Codex CLI 0.145.0 both report cached input as a subset of
  `input_tokens`; the runner never adds it a second time.
- `total_tokens = input_tokens + output_tokens` is the primary model-work measure.
- A separate sensitivity metric weights cached input at 0.25. It is clearly labelled and is not
  presented as OpenAI's proprietary weekly-quota formula.
- The runner reads the fresh weekly percentage returned in each Codex response and fails closed if
  it cannot recover quota telemetry. `--max-weekly-increase-pct` defaults to 30 percentage points.
- Pair order is seeded and alternates which arm runs first, reducing a simple time/order bias.
- A process exit of zero is insufficient: the independent verifier must also pass.

`--raw-profile native` measures the user's real installed Codex CLI harness. The optional
`reduced-config` profile invokes Codex's own user-config/rules/plugin isolation flags. Codex 0.145.0
still loads the installed skill catalog in that mode and emits a skills-budget warning, so the
profile is not called "clean."

After a run, produce the authoritative corrected report from the immutable artifacts:

```bash
scripts/harness-bench/rescore_codex_oauth.py --run-dir /path/to/run
```

This replaces Forge's final stream counter with the complete per-session Store ledger (including
post-stream auxiliary memory usage), re-runs the TypeScript contract's strict no-emit build without
requiring a reference-specific `lint` alias, and writes separate `aggregate.corrected.{json,md}`
files. It never overwrites the original live aggregate.

## Official SWE-bench Verified

The controlled fixtures can reach a quality ceiling. Use the separate matched runner plus the
official Docker evaluator for quality-discriminating evidence.

Select a deterministic, repo-diverse sample before running either model arm:

```bash
scripts/harness-bench/select_swe_verified.py \
  --source /path/to/swe-bench-verified.jsonl \
  --out /path/to/swe-verified-stratified.jsonl \
  --seed 20260726 \
  --per-band 2
```

The selector ranks instances independently within the published `<15 min fix`, `15 min - 1 hour`,
and `1-4 hours` bands, and never repeats a repository. The adjacent manifest records the source
hash, algorithm, seed, and selected ids.

Run every selected instance through Forge and native Codex on all three GPT-5.6 models:

```bash
scripts/harness-bench/compare_codex_oauth_swe.py \
  --dataset /path/to/swe-verified-stratified.jsonl \
  --out /path/to/run \
  --worktree-root /path/to/reusable-clones \
  --forge-bin ./target/debug/forge \
  --baseline-weekly-pct 2 \
  --max-weekly-increase-pct 28
```

This runner defaults to regular `high`; pass `--reasoning-effort high`
explicitly in a published run. It uses a one-arm segment and requires an
external quota refresh before resume.
It retains raw JSONL, stderr, patches, process results, and full Forge Store usage, then writes one
standard prediction file per arm/model under `predictions/`. It deliberately leaves
`official_resolution` unset. Forge's internal `.forge/checkpoints/**` snapshots are hidden from
agent-visible Git status and excluded from submitted patches; legitimate project files elsewhere
under `.forge/` remain eligible.

Before the provider starts, the runner:

- clones/checks out the exact official base;
- replaces Git metadata with one synthetic commit having the same tree;
- removes remotes and unreachable upstream objects;
- verifies that only one commit is reachable;
- adds an integrity preamble forbidding network, external solution sources, and
  later Git history;
- records a separate Forge database for each cell; and
- captures both committed and uncommitted changes relative to the synthetic
  base.

Evaluate each prediction file with pinned `swebench` in Docker:

```bash
python -m swebench.harness.run_evaluation \
  --dataset_name /path/to/swe-verified-stratified.jsonl \
  --predictions_path /path/to/run/predictions/forge__gpt-5.6-sol.jsonl \
  --max_workers 2 \
  --run_id forge_sol
```

Only those official reports support a resolved/unresolved claim. A non-empty patch, successful
agent process, or model assertion is never treated as resolution.

After all arm/model reports exist, bind them back to the matched telemetry and generate the
authoritative aggregate:

```bash
scripts/harness-bench/analyze_codex_oauth_swe.py \
  --run-dir /path/to/run \
  --evaluation-dir /path/to/evaluator-working-directory
```

The analyzer fails on missing or duplicate reports, evaluator errors, incomplete outcomes,
unmatched arms, or incomplete token/wall telemetry. It writes `official-analysis.{json,md}` under
the run directory by default.

## Cell audit and clean native baselines

Audit every retained formal run before buying another arm:

```bash
PYTHONPATH=scripts/harness-bench \
python3 scripts/harness-bench/audit_benchmark_cells.py \
  /path/to/run-a /path/to/run-b \
  --out-json /path/to/cell-evidence.json \
  --out-markdown /path/to/cell-ledger.md \
  --codex-native-out /path/to/clean-codex-native.json \
  --claude-native-out /path/to/clean-claude-native.json
```

A reusable cell must have a dataset-verified task and official base, the
declared model and recorded CLI/binary version, the 1,500-second comparable
timeout, regular `high` effort (or genuine mesh auto with no model or effort
override), the integrity preamble, a clean trace, a successful provider
process, a non-empty patch whose byte count and hash match its metadata, and
exactly one official evaluator result whose patch applied. Legacy or
non-isolated suites remain in the ledger but are invalid for the current
headline.

Trace auditing is operation-aware. A URL literal written into source code is
not a network access; an external-search tool or an actual `curl`, `wget`, `gh`,
Git-history, or Git-remote command is.

## Regular full-mesh auto

`compare_mesh_swe.py` consumes only the clean native baseline files emitted by
the auditor. It deduplicates by task and runs Forge once per unique instance.

```bash
PYTHONPATH=scripts/harness-bench \
python3 scripts/harness-bench/compare_mesh_swe.py \
  --dataset /path/to/swe-verified.jsonl \
  --codex-analysis /path/to/clean-codex-native.json \
  --claude-analysis /path/to/clean-claude-native.json \
  --out /path/to/mesh-run \
  --worktree-root /path/to/mesh-worktrees \
  --forge-bin ./target/release/forge \
  --timeout-seconds 1500 \
  --claude-baseline-weekly-pct 23 \
  --claude-max-weekly-increase-pct 5 \
  --observed-claude-weekly-pct 26 \
  --codex-baseline-weekly-pct 26 \
  --codex-max-weekly-increase-pct 10 \
  --observed-codex-weekly-pct 33 \
  --max-new-trials 1
```

The mesh command deliberately has no model or effort flag. The runner clears
inherited `FORGE_MESH__*` overrides and rejects configurations where automatic
discovery, automatic orchestration, or failover is disabled. It hashes the
binary, runner, history-preparation code, configuration, dataset, and native
baseline inputs into the resume manifest. It always stops after one paid arm so
Helm can be refreshed externally before `--resume`.
