# `forge schedule` — recurring headless runs on native OS timers

> **Status: shipped (#470).** The cron analog to `/loop`/`/goal`: register a task once and a
> native OS timer fires `forge run` on it forever — no daemon, no Forge process left running.

## What it does

```bash
forge schedule add "check the deploy, alert me in the summary" --every 30m
forge schedule add "morning standup summary" --at 09:00
forge schedule add "weekly deps audit" --cron "0 8 * * 1" --tz Europe/Amsterdam
forge schedule                       # list (id, spec, task, last run)
forge schedule remove <id-prefix>    # uninstall the timer + delete (git-style prefix ok)
```

Exactly one of `--every` / `--at` / `--cron` selects the schedule:

| Flag | Format | Meaning |
|------|--------|---------|
| `--every` | `<N><unit>`, unit `s`/`m`/`h`/`d` (`30m`, `1h`, `1d`) | fixed interval |
| `--at` | `HH:MM` | once a day at that local time |
| `--cron` | 5-field cron (`0 3 * * 1-5`), translated to systemd / launchd / Task Scheduler; a raw `OnCalendar=` string still works on Linux | calendar schedule |

`--mode` (e.g. `bypass`, `accept-edits`) and `--model` pin the permission mode and model each tick
runs with. The tick runs `forge run "<task>"` in the directory where the schedule was added.

Policy flags on `add` (all optional, also editable in the app):

| Flag | Meaning |
|------|---------|
| `--tz Europe/Amsterdam` | read `--at` / `--cron` on that zone's clock (systemd `OnCalendar=... <zone>`; Linux only) |
| `--max-runtime <min>` | kill a tick that runs longer |
| `--max-runs <n>` | pause the schedule after n ticks (a one-time schedule is a dated cron with `--max-runs 1`) |
| `--retries <0-5>` | re-run a failed tick with doubling backoff (60s base) |
| `--stop-on-error` | pause the schedule when a tick fails |
| `--stop-when "<condition>"` | the run is told to end with `[[schedule:done]]` once the condition holds; the tick then pauses the schedule |
| `--skip-missed` | drop a tick missed while the machine was off (`Persistent=false`) instead of running it once on wake |

`forge schedule run <id>` fires one tick now, even when paused. `forge schedule history [id]` lists
recent runs.

## Run history

Every OS timer launches the hidden `forge schedule tick <id>`, which reads the task, mode, model and
policy from the store at fire time and writes one `schedule_run` row per attempt:
`running` -> `ok` / `failed` / `interrupted` / `skipped`. `interrupted` means the tick process died
before it finished (the next tick, `forge schedule history`, or a daemon read notices the dead pid);
`skipped` means the previous tick was still running. The row carries the id of the session the tick
created, so the app links a result straight to it, plus a short summary or the error.

The daemon exposes this at `GET /api/schedule-runs`, `POST /api/schedules/{id}/run` (Run now),
`POST /api/schedules/{id}/update`, and `POST /api/schedule-runs/{id}/read` / `read-all` for the
results inbox.

## How it works

- **No daemon.** Each schedule installs a real OS-native timer:
  - **Linux**: a systemd `--user` service + timer unit pair (`systemctl --user enable --now`)
  - **macOS**: a launchd agent plist in `~/Library/LaunchAgents` (`launchctl load`)
  - **Windows**: a Task Scheduler task (`schtasks /Create`, wrapped in `cmd /C` to `cd` first)
- Schedules are persisted in the store (`schedule` table, migration 0004) — id, task, cwd, mode,
  model, spec, enabled, last run. The table is deliberately **not** in the portable-metadata set:
  timers are machine-local, so `forge migrate` does not carry them.
- **Install failure rolls back**: if the OS timer can't be installed, the store row is removed —
  no ghost schedules.
- `remove` uninstalls the unit/plist/task first, then deletes the row. Ids resolve by unambiguous
  prefix, git-style.
- The spec round-trips through a stored string form (`every:1800` / `daily:09:05` / `cron:<expr>`).

## Design notes

- All three platform renderers (`render_systemd_service`, `render_systemd_timer`,
  `render_launchd_plist`, `render_schtasks_create_args`) are **pure string functions**, unit-tested
  on every platform; only `run_checked` touches the OS. Platform selection is `cfg!(target_os)` at
  **runtime**, so the whole module compiles everywhere.
- XML-escaping for launchd plists; `cmd /C` wrapper on Windows so the task `cd`s to the project
  before running.

## Limitations

- A raw `OnCalendar=` expression and `--tz` are Linux-only; 5-field cron works everywhere Task Scheduler and launchd can express it.
- Timers installed before the run history existed call `forge run` directly and record nothing until they are paused and resumed once.
- `Run now` uses the daemon's own environment, so it needs the same store the timers use.
- Ticks are headless `forge run` turns — output lands in the session store (see `forge sessions` /
  `forge replay`), not a terminal.
- Timers fire under your user session (systemd `--user` / launchd agent): on Linux, enable
  lingering (`loginctl enable-linger`) if ticks must fire while you're logged out.
