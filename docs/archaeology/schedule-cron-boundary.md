# Code archaeology: schedule cron boundary

## Decision

`commands/schedule/cron.rs` owns strict POSIX cron parsing and translation to systemd, launchd, and Windows scheduler trigger models. `schedule.rs` retains CLI/store orchestration, schedule persistence, native installation, and platform file/process effects.

## Invariants

- Unsupported cron syntax is never approximated.
- POSIX day-of-month/day-of-week OR semantics remain explicit in every native renderer.
- Non-POSIX `OnCalendar` strings keep their Linux pass-through behavior and remain rejected on platforms without an equivalent grammar.

## Verification

Focused parser and renderer tests cover supported cron translations, DOM/DOW union behavior, pass-through, and scheduler-specific limitations. Warnings-denied Clippy and the size guard verify the module extraction.
