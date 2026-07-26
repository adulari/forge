# Forge OAuth vs raw Codex benchmark

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

This runner uses the same `xhigh` model commands and per-trial quota gate as the controlled runner.
It retains raw JSONL, stderr, patches, process results, and full Forge Store usage, then writes one
standard prediction file per arm/model under `predictions/`. It deliberately leaves
`official_resolution` unset. Forge's internal `.forge/checkpoints/**` snapshots are hidden from
agent-visible Git status and excluded from submitted patches; legitimate project files elsewhere
under `.forge/` remain eligible.

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
