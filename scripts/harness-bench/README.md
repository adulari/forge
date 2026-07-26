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
