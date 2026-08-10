# Fleet messaging — `forge send` and `message_session`

Any two daemon-hosted (fleet) sessions can exchange messages, and a shell can message any of them.
Before this, only a parent could message its own subagents inside one session tree; a session
running in another window, or an agent working on a different repo, was unreachable.

Shipped in #976.

## From a shell

```bash
forge send <target> "<message>"           # queue it (default)
forge send <target> "<message>" --steer   # deliver at the target's next turn boundary
```

`<target>` is a session **name** or a **unique id prefix**; an ambiguous prefix is refused rather
than guessed at, and an unknown target is an error rather than a silent drop. `--url` and
`--token` exist for a daemon that is not the local one — both default to the local daemon's
loopback origin on the configured `[remote] port` and the persisted `serve-token`.

## From an agent

The `message_session` virtual tool is advertised to the model **only when the session has fleet
messaging attached** — i.e. when it is hosted by `forge serve`. An ordinary terminal session never
sees the tool, so it cannot try to call something that would fail. It is available on both the
direct path and the CLI bridge.

Arguments mirror the CLI: `target`, `message`, and `mode` (`follow_up` | `steer`).

## Delivery modes

| Mode | When it arrives |
|---|---|
| `follow_up` (default) | Queued; delivered when the target goes idle or its current turn ends. |
| `steer` | Delivered at the target's very next turn boundary, ahead of anything already queued. |

`steer` jumps the queue but **never interrupts a turn that is already streaming** — the target
finishes what it is saying, then takes the steer before its own backlog. That is the same rule the
TUI's own steering follows, so a message from another agent cannot corrupt a reply mid-sentence.

Messages arrive in the target's transcript as a labelled turn — `[message from <sender>] …` —
where the sender is `cli` or the sending session's name/id. The target can always tell it is being
addressed by something other than its user.

## What survives a restart

Pending messages are persisted, so a message sent to an offline or busy target is not lost. The
daemon flushes a target's backlog on **every path that (re)joins the fleet** — create, fork,
merge-respawn, and the post-restart resurrection — so a daemon restart between send and delivery
does not strand anything.

The handoff boundary is worth knowing precisely: "delivered" means handed to the target session's
input channel. A message already sitting in a busy driver's in-memory queue when the daemon is
killed is not itself durable — the same as any ordinary queued follow-up prompt, which has never
been persisted either.

## Limits

| Limit | Value | Why |
|---|---|---|
| Message size | 16 KB (`FLEET_MESSAGE_MAX_BYTES`) | Keeps a fleet message the same order of magnitude as a prompt, rather than an accidental file dump routed through the wrong tool. |
| Pending per sender→target | 8 (`FLEET_MESSAGE_PENDING_CAP`) | Stops one chatty sender accumulating unbounded backlog against an unresponsive target. Deliberately the only rate limit. |

Both are checked against the same connection the insert runs on, so the decision and the write
cannot disagree under concurrency. Exceeding either is a clear error to the sender, never a silent
drop.

## Related

- `docs/features/remote-control.md` — the daemon and fleet the messages travel through.
- `docs/architecture/prime-agent-comparison.md` — why this exists (port #4).
