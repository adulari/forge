# Long-run endurance, November 2026

Why do long tasks stop before they are done, and do this branch's fixes stop that? Measured on
`inferhub::cb/deepseek-v4.1-flash` (harness profile `minimal`, the default for `inferhub::*`).

**Result: under a realistic provider outage the old build finished 0 of 12 long tasks without help. Every
one needed a "continue". The fixed build finished 12 of 12 without any.**

## What the owner's own sessions show

These are read-only aggregate counts from the owner's store, 2026-09-01 to 2026-11-07. The script is
`~/.cache/forge-burn-demo/cont_shape2.py`.

- Of 1,753 human prompts, 89 (5.1%) were short "continue" prompts, spread over 10 sessions.
- Only 20 of those 89 followed a text reply, and 10 of those 20 ended mid-intent ("Let me check X:").
- The other 69 followed a turn that the harness ended without a final reply:
  - 28 ended on a tool result;
  - 26 followed a prompt that never got a reply at all;
  - 15 ended on an assistant message whose tool calls never ran.
- Since 2026-10-23, when the default step caps were removed (#1640), there were 23 continue prompts.
  13 of them followed a prompt with no reply. Those cluster in provider outage windows, for example
  InferHub having no capacity for this route from 10:18 to 14:27 UTC on Nov 6.
- Since Oct 1, InferHub dispatches on this model failed 2 times in 979. The stops are not spread across
  ordinary turns; they bunch up in outages.
- Before this branch, nothing recorded why a turn ended.

## Why the short benchmarks never saw it

The stop-guard A/B ([stop-guards-2026-11.md](stop-guards-2026-11.md)) ran 208 single-prompt attempts and
recorded 0 mid-intent stops. This suite reproduced that first. Fault-free, the old build solved 14 of 14
long tasks, with a median of 19 model calls and no compactions: DeepSeek Flash scripts mechanical work
through `run_code`. The stops the owner hits come from the provider side, so the suite has to put the
provider side in.

## The suite

The kit is `~/.cache/forge-ops/ccbench/endure`. It lives outside the repo, like the rest of the bench kit.

- **Tasks.** Each task is generated offline with a hidden verifier.
  - E1: fix 48 seeded bugs.
  - E4: a 61-unit rename.
  - E9-E12: serial chains in which a stage's spec stays sealed until the previous stage passes, so they
    cannot be batched into one script.
  - Median 21-49 model calls and 60-160 tool calls per attempt.
- **Modes.**
  - `single`: one prompt, never "continue".
  - `autocont`: types the literal `continue` after each segment that did not finish, up to 5 times, and
    records how many it took.
- **Faults (`faults` arm).** A pass-through proxy in front of InferHub, seeded per task and attempt, so
  both builds see the same faults:
  - one 12-minute window of `503 no_capacity` (InferHub's own body), starting after request 3-8;
  - 2% of requests reset;
  - 2% of streams cut mid-reply.
- **Isolation.** Each attempt gets its own HOME, XDG dirs and `FORGE_DB`, runs with
  `--no-publish-to-fleet`, and is checked never to reach the owner's daemon.
- **Per attempt it records:**
  - solved, and the end class;
  - Forge's `stop_reason` and `end_reason`;
  - the shape of the last message;
  - steps, compactions, tokens and wall time;
  - nudges by kind, `update_tasks` calls, and the continues needed.

## Numbers

The old build is c38173c3a, the fixed one 3713ca768. Both use the same seeded faults.

| Run | Attempts | Solved without help | Stopped before done | How it ended | Median wall |
|---|---|---|---|---|---|
| no faults, old build | 14 | 14 | 0 | done | 104 s |
| faults, old build | 12 | 0 | 12 | `provider_error` after ~583 s, last message a tool result (the owner's top shape) | 583 s |
| faults, fixed build | 12 | 12 | 0 | waited out the outage, then finished | 992 s |

Continue rate, measured in `autocont` mode with the same faults:

| Build | Attempts | Continues needed |
|---|---|---|
| old | 6 | 1 in 6 of 6 (every attempt) |
| fixed | 6 | 0 in 6 of 6 |

The fixed build's wall time includes the 720 s it spends waiting out the outage.

## What changed on this branch

- **Outage wait.** A pinned or routed turn keeps probing a down provider, every 5 minutes at most, up to
  `mesh.outage_wait_secs` (8 h). The old behaviour gave up after 600 s, or after about 5 s on a refused
  connection. While it waits, every surface says so ([mesh-routing.md](../features/mesh-routing.md)).
- **Mid-intent and open tasks.** A reply that ends mid-intent, or one that leaves tasks open, is now
  re-driven on every profile. Before, `minimal` silenced both, and an open task list also blocked the
  mid-intent nudge.
- **Retries.** Re-drive passes retry transient provider errors.
- **Unattended daemon sessions.** A session with no client attached is now treated as unattended.
- **Hard stops.** A hard stop now closes its pending tool calls.
- **End reasons.** Every turn stores an end reason. A turn that stops short says why on the TUI, phone and
  desktop, and in a push. See `forge turns` in [session-replay.md](../features/session-replay.md).

## Limits

- **One model.** Only DeepSeek V4.1 Flash on InferHub was run, because project rules limit live tests to
  it. Kimi, GLM and Muse were not run live.
- **Short-context compactions.** Compactions only happened in the forced-cap arms
  ([task-list-2026-11.md](task-list-2026-11.md)). The owner's 217.6k compaction point was never reached in
  these tasks.
- **Small samples.** The fault arms have 6-12 attempts each. The effect is all or nothing (0/12 against 12/12), so the sample size does not decide the result. Smaller differences would need more runs.
- **Not reproduced: the mid-intent text stops.** These are the 20 text-reply stops in the store. The suite
  has no arm that reproduces them, so their fix is covered by unit tests only
  (`crates/forge-core/src/tests/stop_integrity.rs`).
