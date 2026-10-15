# Recall baseline, 2026-10-16

How much of a long session survives compaction and a resume, measured on the behavior before
the compaction rework (compaction exactly as it stood on main: summarizer chosen by the
existing candidate chain, tool results pruned before summarizing, no rehydration). The model
that wrote each summary is not recorded by the bench.

| | |
|---|---|
| Date | 2026-10-16 |
| Forge code | `97b3f11d` (origin/main when the branch was cut); the bench itself is the only addition |
| Model | `inferhub::glm-5.3-flash`, pinned |
| Command | `forge bench recall --json --keep-workdir --price-in 0.15 --price-out 0.5` |
| Scenario | 23 scripted turns, 18 probes (16 questions, 2 behavior), 2 compactions, 1 resume |
| Raw result | [recall-baseline-2026-10.json](artifacts/recall-baseline-2026-10.json) |

The harness is described in [bench-recall.md](../features/bench-recall.md).

## Result

| Category | After compaction | After resume |
|---|---:|---:|
| constraint | 3/3 100.0% | 3/3 100.0% |
| correction | 1/1 100.0% | 1/1 100.0% |
| decision | 3/3 100.0% | 3/3 100.0% |
| location | 2/2 100.0% | 2/2 100.0% |
| error_fix | 2/2 100.0% | 2/2 100.0% |
| pending | 2/2 100.0% | 2/2 100.0% |
| number | 2/3 66.7% | 2/3 66.7% |
| behavior | 2/2 100.0% | 2/2 100.0% |
| **Total** | **17/18 94.4%** | **17/18 94.4%** |
| early facts (planted before compaction #1) | 15/15 100.0% | 15/15 100.0% |
| late facts (planted between the compactions) | 2/3 66.7% | 2/3 66.7% |

Compactions: mid-scenario 134 -> 21 messages, near-end 52 -> 15 messages. No scenario turn
failed. No memory question used a tool (the scores are not inflated by re-reading files).

The one miss is `late_bench_budget` ("the header benchmark must stay under 12 ms for 10,000
headers"), a number planted after the first compaction and folded by the second. Both phases
answered "you never gave a time budget", so the summary dropped it, not the resume. Both behavior
probes passed in both phases: the new function in `src/parser.rs` had no `.unwrap()`, and the
commit subject started with `JIRA-123`.

## Spend

| | |
|---|---:|
| Input tokens | 1,316,543 (1,151,374 cached) |
| Output tokens | 9,733 (2,972 reasoning) |
| Model calls / tool calls | 160 / 39 |
| Cost | $0.0069 |
| Wall time | 1,052 s |

The catalog has no price for this model, so cost is computed from `--price-in 0.15` and
`--price-out 0.5` USD per million tokens, the rate an earlier Forge config recorded for
`glm-5.3-flash`. Treat the dollar figure as an estimate; the token counts are the store's.

## What this does and does not show

- **This is a ceiling, not a stress test, for this model.** At 23 turns and a 1M-token window the
  session never got large, so a capable model's own summary kept every early fact. Recall only
  drops on the late number, where the second compaction summarized a summary. Headroom to show an
  improvement is therefore small: `--turns-scale 3` or later-planted facts are the way to widen it.
- **One run, one model.** Replies are stochastic; a single miss of 18 is within what a rerun can
  change. Use the per-category rows as a guide and rerun before drawing a conclusion from one fact.
- **Config differs from daily use.** The bench runs with its own generated config, not the real
  one: no user system-prompt override, no code index, no discovery catalog, no self-MCP, and only
  the pinned model's provider, context window (1,000,000) and price copied over. With no discovery
  catalog the summarizer chain may differ from a real install's (which can shortlist other
  models), so the summary quality here is not necessarily what a daily session gets.
- **Facts are checked by regex.** The matchers accept any phrasing containing the right file name,
  value or reason keyword; they cannot tell whether the model also kept the surrounding nuance.

Reproduce with the command above (needs an InferHub key in the Forge key store); `--mock` checks
the plumbing without a model.
