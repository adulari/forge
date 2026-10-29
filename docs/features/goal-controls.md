# Feature: steering a `/goal` — pause, resume, edit, clear, and the docks above the composer

> **Status: shipped.** Works in the terminal, the desktop/web app and the phone, on a local
> `forge` and on a `forge serve` daemon alike.

## 1. Problem (JTBD)
> When I leave a `/goal` running, I want to stop it for a moment, change what it is after, or
> see how much of its budget it has used, without killing the run and losing its progress.

Before this a goal could only be set and interrupted. Esc ended it for good, the app showed a
banner derived from a transient CLI note ("goal set — …") with a Stop button, and the plan the
agent worked through was visible only on the Tasks tab.

## 2. Commands

| Command | Effect |
|---|---|
| `/goal <objective> [--gate …] [--max-tokens N] [--max-minutes N]` | Starts a goal, as before. |
| `/goal pause` | The goal stops after the turn in flight; nothing else is started and the evaluator is not paid for. |
| `/goal resume` | Continues a paused goal at its iteration, with its token and time counters and the same gates. |
| `/goal edit <objective>` | Rewords a running goal from its next turn, or a paused one at once. Progress carries over. |
| `/goal clear` | A running goal stops after the turn in flight and is dropped; an ended or paused one is dropped at once. |
| `/goal status`, bare `/goal` | Prints state, turn, tokens against the budget, elapsed time and gates. |

The verbs are whole words. `/goal pause the nightly job` is still a goal; `/goal edit` with no
text is a goal named "edit"; with no goal to edit, `/goal edit <text>` starts a goal whose
objective is "edit <text>". `pause`, `clear`, `status` and `edit` run while a turn is in flight
(they never take the session lock); `resume` waits for the turn boundary like other commands.

Esc is not a pause: it cancels the run, and the goal shows as `cancelled`.

## 3. State

A goal is `active`, `paused`, `budget limit` (a `--max-tokens` or `--max-minutes` ceiling),
`complete` (the evaluator and the gates agree), `failed` (stalled, wedged, iteration ceiling,
gate exhausted, spend cap) or `cancelled`. The record lives on the app (`forge_tui::goal_status`)
and is written by the goal loop; it is projected to every surface:

- the terminal composer border chip: `2/5 · 12.5k/50k tok`, `paused · 2/5`, `complete · 5/5`;
  `pausing` / `stopping` while a request waits for the turn to end;
- `/status`;
- the remote snapshot's `goal` object (`objective`, `state`, `iteration`, `tokens_used`,
  `max_tokens`, `max_minutes`, `elapsed_secs`, `synced_at`, `gates`, `reason`, `pending`),
  additive within protocol 10. A host without it still shows a running goal from the notes, with
  Stop as its only control.

The state is held by the running process: a daemon restart ends the goal run, and the record goes
with it.

## 4. The app

`GoalBanner` under the session header shows the state, the objective, turn, tokens against the
budget (with a bar), elapsed time and gate count, and offers the controls the state allows:
Pause, Edit and Stop while running; Resume, Edit and Clear while paused; Clear once it ended.
Pause, Resume and Clear send the commands above; Stop is the interrupt.

Above the composer two docks stack, ordered by the person:

- **Plan** — `snapshot.tasks`, live from `update_tasks`: done rows dimmed, a pulsing dot for the
  task in progress, hollow circles for the rest, `Plan 2/4` in the header. A long plan is
  windowed around the active task. Folding it leaves the headline task.
- **Spend** — what the session cost, from `GET /api/sessions/{id}/spend`: the total, then each
  model and each subagent with tokens, billed spend and the list-price estimate (`≈$0.80 list`
  when nothing was billed). The totals are the ones `/api/usage?session=` reports. Hidden until
  the session has called a provider, and when the host has no such route (Forge Anywhere does not
  carry it).

Each dock's menu moves it up or down, hides it for this chat, and (Plan, wide windows) moves it
to the side panel. Hiding the Plan dock lasts until the plan changes; hiding Spend lasts until
restored from the "Show spend" link. Collapsed state, order, hidden docks and placement persist on
the device. Settings, Appearance, Thread docks sets the placement: above the composer, or only in
the right panel (Tasks, `Mod+Shift+T`); windows narrower than 1024 px keep the dock above the
composer.
