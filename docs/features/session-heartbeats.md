# Feature: Session heartbeats (`/heartbeat`, `manage_heartbeats`)

> **Status: shipped.** Recurring prompts that re-enter a **live** session. Two kinds share one
> table: the single user-owned heartbeat (`/heartbeat every`) and up to eight agent-created ones
> (the `manage_heartbeats` virtual tool). Neither can touch the other.

## 1. Problem (JTBD)

> I want a long-running session to check back on something at an interval — CI, a deploy, a queue —
> without me sitting there re-typing the same prompt, and without spawning a fresh agent that has
> lost everything the session already knows.

`forge schedule` already exists and does **not** solve this: it spawns a new `forge run` process
off an OS timer, so every firing starts from an empty context. A heartbeat instead resubmits its
prompt into the session that is already running, with its full transcript, mesh routing, tools and
memory intact.

## 2. What it is

A heartbeat is a row in `session_heartbeat` holding a prompt and an interval. When it comes due,
the prompt is submitted **as an ordinary queued turn** — the same FIFO a prompt typed while the
agent was busy goes through.

**It is never injected mid-turn.** Delivery only happens when the session is idle with no turn
running, so a heartbeat can never interleave with, interrupt, or corrupt work in progress.

| | `forge schedule` | session heartbeat |
|---|---|---|
| fires via | OS timer | the live session's own loop |
| runs in | a fresh `forge run` process | the session that is already open |
| context | none — starts cold | the full session transcript |
| survives session exit | yes | no — it belongs to that session |

## 3. Two owners, deliberately separated

| owner | created by | how many | addressed by |
|---|---|---|---|
| `user` | `/heartbeat every` (TUI) | **at most one** per session | — (it is the singleton) |
| `agent` | `manage_heartbeats` virtual tool | up to **8** per session | its `label` |

The model can never modify, pause or clear the user's heartbeat, and the user's `/heartbeat`
commands never touch agent-created ones. This is enforced at the **database boundary**, not just in
application code: a partial unique index on `(session_id) WHERE owner = 'user'` means a second
`/heartbeat every` can only ever *replace* that row, never duplicate it, and
`(session_id, label) WHERE owner = 'agent'` does the same for labels.

## 4. The `/heartbeat` command (the user's own)

```
/heartbeat every <interval> <prompt>   set or replace it — e.g. /heartbeat every 5m check the CI status
/heartbeat            (or status)      whether one is set, plus its next-due countdown
/heartbeat pause                       stop firing, keep the prompt and interval
/heartbeat resume                      start firing again, rescheduled from now
/heartbeat clear                       delete it
```

`every` **replaces** rather than stacks, so there is no way to accumulate duplicates by repeating
the command.

### Intervals

`30s`, `5m`, `1h` — a number plus an `s`/`m`/`h` suffix. The minimum is **30 seconds**; below that
it stops being a recurring prompt and becomes a busy-loop that could dominate the session's turn
budget.

**A bare number is rejected on purpose.** `/heartbeat every 30 check CI` is an error rather than a
silent guess, because guessing wrong between 30 seconds and 30 minutes is exactly the kind of typo
that would quietly burn a budget.

## 5. The `manage_heartbeats` tool (the agent's own)

Advertised to the model on both the direct path and the CLI-bridge `mcp-serve` handler, so a
bridged claude/codex sees it too. Actions: `create` (needs `label`, `prompt`, `interval`), `list`,
`pause`, `resume`, `delete` (each needs `label`).

Capped at 8 per session. The user's singleton is separate and never counts against that cap.

## 6. Delivery semantics

**Claiming is atomic.** `claim_due_heartbeats` advances `next_due_at` to `now + interval_secs` in
the *same statement* that claims the row, so a crash or restart between claiming and delivering
cannot double-deliver.

**Missed ticks coalesce.** A heartbeat that came due repeatedly during a long busy stretch
reschedules from `now`, never from its stale `next_due_at`. It fires **once** as a catch-up rather
than replaying a backlog of every tick it missed — which for a 30s heartbeat across a 40-minute
turn would otherwise be 80 queued prompts.

**Resuming reschedules from now**, so a heartbeat paused for hours does not fire the instant it
comes back.

**Where it runs:** `try_deliver_due_heartbeats` is shared by the interactive TUI loop and the
daemon-hosted driver loop, and is called both right after a turn ends and on a coarse periodic
tick — so a heartbeat still fires in a session sitting fully idle. It takes the session lock with
`try_lock`: on contention (a turn is starting elsewhere right now) it skips and retries on the next
call rather than blocking the render loop.

## 7. Storage

Migration **#28** adds `session_heartbeat`. Like `schedule` and `queue_task`, it is **local machine
state and deliberately not in `PORTABLE_METADATA_TABLES`** — a heartbeat belongs to a session on
this machine, so it does not sync to other devices.

There are no `[config]` keys: a heartbeat is per-session state, created and removed at runtime.

## 8. Definition of done
- [x] One user heartbeat per session, enforced by a partial unique index rather than by convention.
- [x] `/heartbeat every | status | pause | resume | clear`.
- [x] `manage_heartbeats` virtual tool with a per-session cap, on the direct path and the bridge.
- [x] Interval parsing with a required unit suffix and a 30s floor.
- [x] Delivery as an ordinary queued turn, never mid-turn.
- [x] Atomic claim (no double-delivery across a restart) and coalesced catch-up.
- [x] Fires in both the interactive TUI and the daemon driver, including while fully idle.
