# Stop-guard A/B, November 2026

The four `[harness]` stop-guard flags ([harness-profile.md](../features/harness-profile.md#harness-stop-guard-flags-ab-default-off))
have been opt-in since 3cb7a723b, waiting for a measurement. This is that measurement for
DeepSeek V4.1 Flash on Forge's native loop.

**Result: keep the defaults (all off) for this model.** No arm produced a single reply that ended
on announced-but-undone work, so there was nothing for the guards to fix, and none of them raised
the pass rate.

## Setup

- Model `inferhub::cb/deepseek-v4.1-flash` (harness profile `minimal`, the built-in default for
  `inferhub::*`, so with the flags off even the mid-intent nudge is silenced).
- Suites a, b and d of the parity kit (26 tasks: fixes, refactors, long-horizon edits, debugging,
  interactive and background commands), 2 attempts per arm, 208 attempts in total.
- Forge 2.27.0, each attempt in its own temp HOME, config and store (`ccbench` runner2, arms differ
  only in the `[harness]` block appended to the seed's config).
- Run 2026-11-06 14:27-16:40 UTC. InferHub had no capacity for this route from 10:18 to 14:27 UTC,
  so the run started late; no attempt hit an outage once it started.

"Mid-intent" is Forge's own classifier (`crates/forge-core/src/mid_intent.rs`, the one the nudge
uses) applied to each attempt's final assistant reply. A Python port of it scores 68/68 positives
and 114/114 negatives on the classifier's labelled corpus.

## Numbers

| Arm | Solved | Mid-intent final replies | Median tokens | Median wall | Median model calls |
| --- | --- | --- | --- | --- | --- |
| off (default) | 47/52 (24 + 23) | 0 | 208k | 68.7 s | 12 |
| `stop_retries` | 45/52 (22 + 23) | 0 | 187k | 66.1 s | 11.5 |
| `completion_contract` | 47/52 (23 + 24) | 0 | 190k | 70.3 s | 11.5 |
| all four | 46/52 (23 + 23) | 0 | 199k | 83.8 s | 12 |

Pass-rate differences are one or two tasks out of 52, inside attempt-to-attempt noise (the same arm
moves by one task between attempts). The token differences are under 10% and also not stable
across the two attempts. All four flags together cost 22% more median wall time for no gain.

## What this does and does not show

- The owner's own "continue" prompts come from long, multi-turn sessions (one November session
  accounts for most of them, on a different route of the same model). Single-prompt suites do not
  reproduce that pattern: here the model finishes or fails, it does not stop mid-sentence.
- So this rules out turning the flags on to fix single-task completion, but it does not measure
  them where the stops actually happen. The next useful measurement is a multi-turn suite (the
  `multiturn` kit in ccbench) or replaying the shape of a long session with mock tools.

The scoring script and the raw per-attempt rows live outside the repo
(`~/.cache/forge-burn-demo/sg_score.py`, `~/.cache/forge-ops/ccbench/results/runs/sg-*-abd`).
