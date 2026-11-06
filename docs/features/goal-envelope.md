# Goal spend envelopes

`/goal <objective> --budget <spec>` caps what one goal may spend. The goal pauses, resumable, when the envelope is used.

| Spec | Meaning |
| --- | --- |
| `claude:20%w` | 20 points of the Claude weekly window, counted from the reading when the goal started |
| `codex:15%w` | the same for Codex |
| `claude:30%5h` | 30 points of the 5-hour window |
| `$5` | session cost since the goal started; also enforced inside a turn |

- The baseline is read before the first turn. If no fresh reading exists the goal does not start.
- The check runs at each turn boundary against the store's quota rows, refreshed through the claude/codex probes when older than two minutes. A reading older than ten minutes cannot verify the envelope, so the goal pauses and says so rather than spending blind.
- A window that rolls over mid-goal carries what was used before the rollover.
- `/goal budget 30%w` (or `$10`) raises the slice on a running or paused goal. `/goal resume` is refused while the envelope is spent, unless the window has reset (the next run then counts from a fresh baseline).
- Auto-resume at the window reset is not implemented; the pause message gives the reset time.
- A paused or running envelope goal is mirrored to `session.goal_state` (migration 53) by the daemon driver and comes back paused after a restart.
- The meter (`envelope 12/20% claude weekly`) shows on the composer chip, `/goal status`, `Snapshot.goal.envelope`, and the phone/desktop goal banner.
- A headless `forge run "/goal ... --budget"` refuses the flag; use `--max-tokens`/`--max-minutes`.

Test-only: in debug builds `FORGE_TEST_GOAL_ENVELOPE_QUOTA=<file>` with a line `claude weekly 42.5 [resets_unix]` stands in for a live reading, re-read at every check.
