//! `forge schedule` subcommands.

use clap::Subcommand;

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum ScheduleCmd {
    /// Register a recurring task and install the native OS timer that fires it. Exactly one of
    /// `--every` / `--at` / `--cron` selects the schedule.
    ///
    /// Examples:
    ///   forge schedule add "check the deploy" --every 30m
    ///   forge schedule add "morning standup summary" --at 09:00
    Add {
        /// The prompt / task each tick runs (quote it).
        task: Vec<String>,
        /// Fire on a fixed interval: `<N><unit>` with unit s/m/h/d, e.g. `30m`, `1h`, `1d`.
        #[arg(long, conflicts_with_all = ["at", "cron"])]
        every: Option<String>,
        /// Fire once a day at this local time (`HH:MM`).
        #[arg(long, conflicts_with_all = ["every", "cron"])]
        at: Option<String>,
        /// Raw OS-native calendar expression (systemd `OnCalendar=` syntax on Linux; not
        /// supported on macOS/Windows).
        #[arg(long, conflicts_with_all = ["every", "at"])]
        cron: Option<String>,
        /// Permission mode passed to each `forge run` tick (e.g. `bypass`, `accept-edits`).
        #[arg(long)]
        mode: Option<String>,
        /// Pin a specific model for each tick.
        #[arg(long)]
        model: Option<String>,
        /// IANA time zone the `--at` / `--cron` calendar is read in (default: this machine's).
        /// Needs systemd, so Linux only.
        #[arg(long)]
        tz: Option<String>,
        /// Kill a tick that runs longer than this many minutes.
        #[arg(long, value_name = "MINUTES")]
        max_runtime: Option<i64>,
        /// Pause the schedule after this many ticks.
        #[arg(long, value_name = "N")]
        max_runs: Option<i64>,
        /// Re-run a failed tick up to this many times (0-5), with doubling backoff.
        #[arg(long, default_value_t = 0)]
        retries: i64,
        /// Pause the schedule when a tick fails.
        #[arg(long)]
        stop_on_error: bool,
        /// Pause the schedule once the run judges this condition true (e.g. "CI is green").
        #[arg(long, value_name = "CONDITION")]
        stop_when: Option<String>,
        /// Drop a tick missed while the machine was off instead of running it once on wake.
        #[arg(long)]
        skip_missed: bool,
    },
    /// List registered schedules.
    List,
    /// Start one tick of a schedule now (even when paused) and wait for it to finish.
    Run {
        /// Schedule id, or a prefix of one.
        id: String,
    },
    /// Show recent runs: status, session, and what happened.
    History {
        /// Limit to one schedule (id or prefix).
        id: Option<String>,
        /// How many runs to show.
        #[arg(long, default_value_t = 20)]
        limit: i64,
    },
    /// Uninstall a schedule's OS timer and delete it (id or unambiguous prefix).
    Remove {
        /// Schedule id, or a prefix of one (see `forge schedule list`).
        id: String,
    },
    /// Run one tick of a schedule now and record it in its run history. This is what the OS
    /// timers invoke.
    #[command(hide = true)]
    Tick {
        /// Schedule id, or a prefix of one.
        id: String,
        /// The tick was started by hand ("Run now") rather than by the timer, so a paused
        /// schedule still runs.
        #[arg(long)]
        manual: bool,
    },
}
