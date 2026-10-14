# Feature: harness profile — how much advisory steering a model gets

> Status: **SHIPPED** (2026-10-14). Code: `crates/forge-config/src/harness_profile.rs` (config +
> glob resolution), `crates/forge-core/src/harness_profile.rs` (the per-step switch),
> `crates/forge-core/src/empty_reply.rs` (the once-per-streak empty-reply nudge). Tests:
> `crates/forge-core/src/tests/harness_profile.rs`.

## 1. Problem

Forge has about twenty nudge and guard modules. Most of them were tuned on weak or free models,
which narrate instead of acting, stop with tasks open, or go quiet for half an hour. Frontier
models mostly do not need that help, and some of it may cost them: every nudge is an extra
message in the context, and some (the continue nudge, the mid-intent nudge) re-drive the whole
context for another round. Nobody has measured whether the advisory layer helps or hurts the
solve rate on strong models. You need a switch before you can measure it.

## 2. What ships

```toml
[mesh]
harness_profile = "guarded"        # default; current behaviour

[mesh.harness_profiles]            # per-model overrides; `*` matches any run of characters
"bedrock::*" = "minimal"
"codex-oauth::gpt-5.6-luna" = "minimal"
```

When more than one glob matches, the one with the most literal characters wins, so
`bedrock::*kimi*` beats `bedrock::*`. `/config` shows `mesh.harness_profile` with both options.
Globs can only be edited in `$EDITOR`.

Built-in default: InferHub models (`inferhub::*`) run `minimal` unless a `[mesh.harness_profiles]`
glob matches them. A 72-run ablation on `inferhub::glm-5.3-flash` (6 tasks × 3 attempts per setup)
measured 14% fewer input tokens and 17% fewer model calls with no pass-rate change. Opt out with
`"inferhub::*" = "guarded"`.

The profile is resolved for the model that is actually driving the turn. It is set once from the
routed model at turn start, which covers the turn-start injections, and again after every model
response, so a failover onto a model with a different profile switches it for the rest of the
turn.

### `minimal` turns off the advisory nudges

| Nudge | Module | Under `minimal` |
|---|---|---|
| Progress note after a silent tool stretch | `silence_guard.rs` | The model gets no ask. The user still sees the harness-written `Progress` line. |
| Stall / stagnation nudge | `loop_progress.rs` (`STALL_NUDGE`, `STAGNATION_NUDGE`) | No message. The tracker keeps counting, so the relapse **halt** fires on the same schedule. |
| Commit / push reminders | `git_hygiene.rs` | Neither the turn-start nor the mid-turn `[git]` line is sent. |
| Auto-orchestrate prompt | `lib.rs` (`mesh.auto_orchestrate`) | Not injected. |
| Mid-intent nudge | `mid_intent.rs` | A reply that ends on an announced action stands as the answer. |
| Continue nudge (direct path) | `nudge_policy.rs` via `model_loop.rs` | With tasks still open, the text reply is accepted rather than re-driven. |

### `minimal` keeps the safety and integrity guards

The permission broker, the doom-loop halt (and the "change approach" nudge that comes before it),
the failure-loop halt, the stagnation halt, the phantom-edit gate (it now also runs when tasks are
open and the continue nudge is off), the tool-call-as-text repair, the completion-verification
gate, the step cap, the turn and session spend caps, the per-turn input-token ceiling and the
empty-reply recovery all stay on. The tests in `tests/harness_profile.rs` prove each advisory
nudge is off and each of these guards still fires under `minimal`.

### Not covered yet

- **CLI bridges.** The bridge re-drive with open tasks, and its one stall escalation, stay on under
  every profile. A bridge is a one-shot subprocess, so re-driving it is how a tracked plan gets
  finished at all. That makes it closer to the model's own loop than to advice.
- **Stalled tasks** (`task_staleness.rs`) stay on: they keep the task list honest for the
  completion gate, which is kept.

## 3. Empty-reply nudge: once per empty streak

Measured over 60 days, only 11 of 59 calls that followed "Your last response was empty" made
progress (19%), and each one re-sent the full context. The model now gets **one** such nudge per
streak of empty replies, shared across every model loop and failover in the turn. Before this it
got two per loop, and the count also reset on every failover. If the model answers the nudge with
another empty reply, the turn goes straight to the honest-failure path that already existed: the
model is benched, the turn fails over to the next model in the chain, and on a pinned turn it stops
with the pin named.

The nudge re-arms only when the model makes tool progress again. A strict once-per-turn cap would
bench a model, or stop a pinned turn outright, over one transient empty reply a hundred steps after
an unrelated one. A model that fails over gets no fresh nudge: its first empty reply benches it.
This applies to every profile.

## 4. How to evaluate

Run `forge bench swe --attempts 3` twice per model, once with `harness_profile = "guarded"` and
once with `"minimal"`, on Claude through Bedrock and on codex luna. Keep `minimal` for a model only
where mean pass@1 holds and steps or cost go down.
