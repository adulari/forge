# `forge schedule` — recurring headless runs on native OS timers

> **Status: shipped (#470).** The cron analog to `/loop`/`/goal`: register a task once and a
> native OS timer fires `forge run` on it forever — no daemon, no Forge process left running.

## What it does

```bash
forge schedule add "check the deploy, alert me in the summary" --every 30m
forge schedule add "morning standup summary" --at 09:00
forge schedule add "weekly deps audit" --cron "Mon 08:00"   # Linux only
forge schedule                       # list (id, spec, task, last run)
forge schedule remove <id-prefix>    # uninstall the timer + delete (git-style prefix ok)
```

Exactly one of `--every` / `--at` / `--cron` selects the schedule:

| Flag | Format | Meaning |
|------|--------|---------|
| `--every` | `<N><unit>`, unit `s`/`m`/`h`/`d` (`30m`, `1h`, `1d`) | fixed interval |
| `--at` | `HH:MM` | once a day at that local time |
| `--cron` | raw systemd `OnCalendar=` expression | calendar schedule — **Linux only**, rejected cleanly on macOS/Windows |

`--mode` (e.g. `bypass`, `accept-edits`) and `--model` pin the permission mode and model each tick
runs with. The tick runs `forge run "<task>"` in the directory where the schedule was added.

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

- `--cron` is systemd `OnCalendar` syntax and Linux-only; use `--every`/`--at` elsewhere.
- Ticks are headless `forge run` turns — output lands in the session store (see `forge sessions` /
  `forge replay`), not a terminal.
- Timers fire under your user session (systemd `--user` / launchd agent): on Linux, enable
  lingering (`loginctl enable-linger`) if ticks must fire while you're logged out.
