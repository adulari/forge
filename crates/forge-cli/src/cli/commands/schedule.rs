//! `forge schedule` — register/list/remove recurring tasks that fire `forge run` via a native OS
//! timer (systemd `--user` on Linux, launchd on macOS, Task Scheduler on Windows). Headless analog
//! to `/loop`/`/goal`: there is no long-lived Forge process — the OS scheduler drives each tick by
//! re-invoking the `forge` binary, exactly like a cron job.

use anyhow::{Context, Result};

use crate::*;

pub(crate) fn schedule_cmd(cmd: Option<ScheduleCmd>) -> Result<()> {
    match cmd {
        None | Some(ScheduleCmd::List) => list_schedules_cmd(),
        Some(ScheduleCmd::Add {
            task,
            every,
            at,
            cron,
            mode,
            model,
        }) => add_schedule_cmd(task.join(" "), every, at, cron, mode, model),
        Some(ScheduleCmd::Remove { id }) => remove_schedule_cmd(&id),
    }
}

// ---------------------------------------------------------------------------
// Schedule spec — a small enum instead of a full cron parser. `Every`/`Daily` cover the common
// cases and render per-OS below; `Cron` carries a calendar expression in one of two dialects:
// standard 5-field POSIX cron (`0 6 * * 1`), which is translated to each OS's native trigger by
// [`parse_posix_cron`] + the `cron_to_*` renderers, or — when the string is not valid POSIX cron —
// systemd's own `OnCalendar=` grammar (`Mon *-*-* 06:00:00`), passed through verbatim as it always
// was. The pass-through is what keeps schedules created before the translator existed working.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ScheduleSpec {
    Every(std::time::Duration),
    Daily { hour: u32, minute: u32 },
    Cron(String),
}

impl ScheduleSpec {
    /// Serialize to the `schedule.cron` column's TEXT format.
    pub(crate) fn to_stored(&self) -> String {
        match self {
            ScheduleSpec::Every(d) => format!("every:{}", d.as_secs()),
            ScheduleSpec::Daily { hour, minute } => format!("daily:{hour:02}:{minute:02}"),
            ScheduleSpec::Cron(expr) => format!("cron:{expr}"),
        }
    }

    /// Parse the `schedule.cron` column back into a spec (round-trips with [`Self::to_stored`]).
    pub(crate) fn from_stored(s: &str) -> Option<Self> {
        if let Some(rest) = s.strip_prefix("every:") {
            return rest
                .parse::<u64>()
                .ok()
                .map(|secs| ScheduleSpec::Every(std::time::Duration::from_secs(secs)));
        }
        if let Some(rest) = s.strip_prefix("daily:") {
            let (h, m) = rest.split_once(':')?;
            return Some(ScheduleSpec::Daily {
                hour: h.parse().ok()?,
                minute: m.parse().ok()?,
            });
        }
        s.strip_prefix("cron:")
            .map(|expr| ScheduleSpec::Cron(expr.to_string()))
    }

    /// Human summary for `add`'s confirmation line and `list`.
    ///
    /// `Cron` names the dialect that was actually recognised, because the two behave differently:
    /// a POSIX expression is translated to a native trigger (and the desktop can compute a real
    /// "next run" from it), while an OnCalendar string is handed to systemd untouched and is not
    /// installable on macOS/Windows at all. Labelling both "cron" would hide that.
    pub(crate) fn describe(&self) -> String {
        match self {
            ScheduleSpec::Every(d) => format!("every {}", fmt_duration_human(*d)),
            ScheduleSpec::Daily { hour, minute } => format!("daily at {hour:02}:{minute:02}"),
            ScheduleSpec::Cron(expr) if parse_posix_cron(expr).is_some() => {
                format!("cron `{expr}`")
            }
            ScheduleSpec::Cron(expr) => format!("OnCalendar `{expr}`"),
        }
    }
}

/// Parse `--every` shorthand: `<N><unit>` with unit s/m/h/d (e.g. `30m`, `1h`, `1d`).
pub(crate) fn parse_every(spec: &str) -> Result<std::time::Duration> {
    let trimmed = spec.trim();
    let bad =
        || anyhow::anyhow!("invalid --every value '{spec}' — expected e.g. `30m`, `1h`, `1d`");
    if trimmed.len() < 2 {
        return Err(bad());
    }
    let (num, unit) = trimmed.split_at(trimmed.len() - 1);
    let n: u64 = num.parse().map_err(|_| bad())?;
    let secs = match unit {
        "s" => n,
        "m" => n * 60,
        "h" => n * 3600,
        "d" => n * 86_400,
        _ => return Err(bad()),
    };
    if secs == 0 {
        anyhow::bail!("--every must be greater than zero");
    }
    Ok(std::time::Duration::from_secs(secs))
}

/// Parse `--at "HH:MM"` into a 24h hour/minute pair.
pub(crate) fn parse_at(spec: &str) -> Result<(u32, u32)> {
    let (h, m) = spec
        .split_once(':')
        .ok_or_else(|| anyhow::anyhow!("--at must be HH:MM, got '{spec}'"))?;
    let hour: u32 = h
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid hour in --at '{spec}'"))?;
    let minute: u32 = m
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid minute in --at '{spec}'"))?;
    if hour > 23 || minute > 59 {
        anyhow::bail!("--at '{spec}' is out of range (00:00–23:59)");
    }
    Ok((hour, minute))
}

fn fmt_duration_human(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    if secs > 0 && secs.is_multiple_of(86_400) {
        format!("{}d", secs / 86_400)
    } else if secs > 0 && secs.is_multiple_of(3600) {
        format!("{}h", secs / 3600)
    } else if secs > 0 && secs.is_multiple_of(60) {
        format!("{}m", secs / 60)
    } else {
        format!("{secs}s")
    }
}

// ---------------------------------------------------------------------------
// forge schedule add / list / remove
// ---------------------------------------------------------------------------

fn add_schedule_cmd(
    task: String,
    every: Option<String>,
    at: Option<String>,
    cron: Option<String>,
    mode: Option<String>,
    model: Option<String>,
) -> Result<()> {
    if task.trim().is_empty() {
        anyhow::bail!("empty task — usage: forge schedule add \"<task>\" --every 30m");
    }
    let spec = match (every, at, cron) {
        (Some(e), None, None) => ScheduleSpec::Every(parse_every(&e)?),
        (None, Some(a), None) => {
            let (hour, minute) = parse_at(&a)?;
            ScheduleSpec::Daily { hour, minute }
        }
        (None, None, Some(c)) => ScheduleSpec::Cron(c),
        (None, None, None) => {
            anyhow::bail!("pass one of --every / --at / --cron, e.g. `--every 30m`")
        }
        _ => unreachable!("clap's conflicts_with_all rules out combining --every/--at/--cron"),
    };

    let cwd = std::env::current_dir().context("resolving current directory")?;
    let cwd = cwd.to_string_lossy().to_string();
    let forge_exe = std::env::current_exe()
        .context("resolving the forge binary path")?
        .to_string_lossy()
        .to_string();

    let store = open_store()?;
    let id = forge_types::new_id();
    store
        .add_schedule(
            &id,
            &task,
            &cwd,
            mode.as_deref(),
            model.as_deref(),
            &spec.to_stored(),
        )
        .context("persisting schedule")?;

    // Install the OS timer only after the row lands — on failure, roll the row back so a failed
    // `add` never leaves an orphaned schedule with no matching timer.
    if let Err(e) = install_timer(
        &id,
        &spec,
        &task,
        &cwd,
        mode.as_deref(),
        model.as_deref(),
        &forge_exe,
    ) {
        let _ = store.remove_schedule(&id);
        return Err(e).context("installing the OS timer");
    }

    println!(
        "✓ scheduled ({}) in {cwd}\n  task: {task}\n  id: {}",
        spec.describe(),
        &id[..id.len().min(8)]
    );
    Ok(())
}

fn list_schedules_cmd() -> Result<()> {
    let store = open_store()?;
    let rows = store.list_schedules().context("listing schedules")?;
    if rows.is_empty() {
        println!("no schedules registered — `forge schedule add \"<task>\" --every 30m`");
        return Ok(());
    }
    println!(
        "{:<10} {:<7} {:<16} {:<9} {:<30} TASK",
        "ID", "ENABLED", "SCHEDULE", "LAST RUN", "CWD"
    );
    for s in &rows {
        let id: String = s.id.chars().take(8).collect();
        let spec = ScheduleSpec::from_stored(&s.cron)
            .map(|sp| sp.describe())
            .unwrap_or_else(|| s.cron.clone());
        let last_run = s
            .last_run
            .map(fmt_age)
            .unwrap_or_else(|| "never".to_string());
        println!(
            "{:<10} {:<7} {:<16} {:<9} {:<30} {}",
            id,
            if s.enabled { "yes" } else { "no" },
            spec,
            last_run,
            s.cwd,
            s.task
        );
    }
    Ok(())
}

fn remove_schedule_cmd(id_prefix: &str) -> Result<()> {
    let store = open_store()?;
    let id = resolve_schedule_id(&store, id_prefix)?;
    uninstall_timer(&id)?;
    store.remove_schedule(&id).context("deleting schedule")?;
    println!("✓ removed schedule {}", &id[..id.len().min(8)]);
    Ok(())
}

fn resolve_schedule_id(store: &Store, prefix: &str) -> Result<String> {
    let mut matches = store
        .matching_schedule_ids(prefix)
        .context("looking up schedule")?;
    match matches.len() {
        0 => anyhow::bail!("no schedule matching '{prefix}' — see `forge schedule list`"),
        1 => Ok(matches.remove(0)),
        n => anyhow::bail!("'{prefix}' is ambiguous ({n} schedules match) — use more characters"),
    }
}

// ---------------------------------------------------------------------------
// POSIX cron → native calendar triggers.
//
// `--cron` used to mean "systemd OnCalendar=", so `cron:0 6 * * 1` — what everyone actually types,
// and what the desktop's cadence field advertises — was written straight into a unit that systemd
// then refused to parse. The parser below understands the standard 5-field grammar and each OS's
// renderer turns it into a native trigger; anything it does not fully understand falls back to the
// old verbatim behaviour so existing rows are untouched.
//
// It deliberately mirrors `mobile/src/lib/cron.ts` field for field (same range rules, same
// `a/n`-runs-to-the-end reading, same 7→0 Sunday fold, same Vixie star flags taken from the field's
// first character). That file computes the "next run" the desktop shows; if the two disagreed the
// UI would advertise a time the installed timer never fires.
// ---------------------------------------------------------------------------

/// The set of instants a parsed expression matches, per field. `*_star` mirrors Vixie cron's
/// DOM_STAR/DOW_STAR flags, which decide whether day-of-month and day-of-week AND or OR.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CronFields {
    minutes: Vec<u32>,
    hours: Vec<u32>,
    days_of_month: Vec<u32>,
    months: Vec<u32>,
    /// Normalised to 0-6; cron's 7 (Sunday) is folded onto 0.
    days_of_week: Vec<u32>,
    dom_star: bool,
    dow_star: bool,
}

impl CronFields {
    /// True when day-of-month and day-of-week are BOTH restricted, which is the one case where
    /// POSIX cron ORs them (`0 0 1 * MON` = the 1st *or* any Monday) instead of ANDing.
    fn day_fields_or(&self) -> bool {
        !self.dom_star && !self.dow_star
    }
}

const CRON_MONTH_NAMES: [(&str, u32); 12] = [
    ("JAN", 1),
    ("FEB", 2),
    ("MAR", 3),
    ("APR", 4),
    ("MAY", 5),
    ("JUN", 6),
    ("JUL", 7),
    ("AUG", 8),
    ("SEP", 9),
    ("OCT", 10),
    ("NOV", 11),
    ("DEC", 12),
];

const CRON_DOW_NAMES: [(&str, u32); 7] = [
    ("SUN", 0),
    ("MON", 1),
    ("TUE", 2),
    ("WED", 3),
    ("THU", 4),
    ("FRI", 5),
    ("SAT", 6),
];

/// The `@`-macros every cron implementation accepts. `@reboot` has no calendar meaning and stays
/// unsupported (it falls through to the OnCalendar pass-through, where systemd rejects it loudly).
const CRON_MACROS: [(&str, &str); 7] = [
    ("@yearly", "0 0 1 1 *"),
    ("@annually", "0 0 1 1 *"),
    ("@monthly", "0 0 1 * *"),
    ("@weekly", "0 0 * * 0"),
    ("@daily", "0 0 * * *"),
    ("@midnight", "0 0 * * *"),
    ("@hourly", "0 * * * *"),
];

fn parse_cron_value(raw: &str, names: Option<&[(&str, u32)]>) -> Option<u32> {
    if !raw.is_empty() && raw.bytes().all(|b| b.is_ascii_digit()) {
        return raw.parse().ok();
    }
    let names = names?;
    let upper = raw.to_ascii_uppercase();
    names.iter().find(|(n, _)| *n == upper).map(|(_, v)| *v)
}

/// One comma-separated field into the explicit list of values it matches, or `None` if any term is
/// malformed. Strict on purpose — a field we do not fully understand must fall back to the raw
/// pass-through rather than install a timer we guessed at.
fn parse_cron_field(
    field: &str,
    min: u32,
    max: u32,
    names: Option<&[(&str, u32)]>,
) -> Option<Vec<u32>> {
    let mut hit: Vec<u32> = Vec::new();
    for term in field.split(',') {
        let mut parts = term.splitn(3, '/');
        let spec = parts.next()?;
        let step_raw = parts.next();
        if parts.next().is_some() {
            return None;
        }
        let step = match step_raw {
            None => 1,
            Some(s) => {
                if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                match s.parse::<u32>() {
                    Ok(0) | Err(_) => return None,
                    Ok(n) => n,
                }
            }
        };

        let (from, to) = if spec == "*" {
            (min, max)
        } else {
            let mut bounds = spec.splitn(3, '-');
            let start = parse_cron_value(bounds.next()?, names)?;
            let end = bounds.next();
            if bounds.next().is_some() {
                return None;
            }
            match end {
                Some(e) => (start, parse_cron_value(e, names)?),
                // A bare `a` is itself; `a/n` means "from a to the end of the field", which is how
                // every widely used cron reads it. Reversed ranges (`FRI-MON`) are an error in
                // Vixie cron rather than a wrap, so they fail the bounds check below.
                None => (start, if step_raw.is_none() { start } else { max }),
            }
        };
        if from < min || to > max || from > to {
            return None;
        }
        let mut value = from;
        while value <= to {
            if !hit.contains(&value) {
                hit.push(value);
            }
            value = value.saturating_add(step);
        }
    }
    hit.sort_unstable();
    Some(hit)
}

/// Parse a 5-field expression (`minute hour day-of-month month day-of-week`) or an `@`-macro.
/// `None` for anything not fully understood — never a partial guess.
pub(crate) fn parse_posix_cron(expression: &str) -> Option<CronFields> {
    let trimmed = expression.trim();
    if trimmed.is_empty() {
        return None;
    }
    let expanded: &str = if trimmed.starts_with('@') {
        CRON_MACROS
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(trimmed))
            .map(|(_, expansion)| *expansion)?
    } else {
        trimmed
    };

    let fields: Vec<&str> = expanded.split_whitespace().collect();
    if fields.len() != 5 {
        return None;
    }

    let minutes = parse_cron_field(fields[0], 0, 59, None)?;
    let hours = parse_cron_field(fields[1], 0, 23, None)?;
    let days_of_month = parse_cron_field(fields[2], 1, 31, None)?;
    let months = parse_cron_field(fields[3], 1, 12, Some(&CRON_MONTH_NAMES))?;
    // 0 and 7 are both Sunday, so the field is parsed over 0-7 and folded onto 0-6 afterwards.
    let raw_dow = parse_cron_field(fields[4], 0, 7, Some(&CRON_DOW_NAMES))?;
    let mut days_of_week: Vec<u32> = raw_dow.iter().map(|d| d % 7).collect();
    days_of_week.sort_unstable();
    days_of_week.dedup();

    Some(CronFields {
        minutes,
        hours,
        days_of_month,
        months,
        days_of_week,
        // Vixie sets the star flag from the field's *first character*, so `*/2` counts as starred
        // and still ANDs. Mirrored rather than "fixed" — this is the reference behaviour.
        dom_star: fields[2].starts_with('*'),
        dow_star: fields[4].starts_with('*'),
    })
}

fn cron_field_is_full(values: &[u32], min: u32, max: u32) -> bool {
    values.len() as u32 == max - min + 1
}

fn all_days_of_month() -> Vec<u32> {
    (1..=31).collect()
}

const ALL_DAYS_OF_WEEK: [u32; 7] = [0, 1, 2, 3, 4, 5, 6];

// --- systemd ---

const SYSTEMD_DOW: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/// A `*`/comma-list component of an `OnCalendar=` expression.
fn on_calendar_component(values: &[u32], min: u32, max: u32) -> String {
    if cron_field_is_full(values, min, max) {
        return "*".to_string();
    }
    values
        .iter()
        .map(|v| format!("{v:02}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn on_calendar_line(fields: &CronFields, days_of_month: &[u32], days_of_week: &[u32]) -> String {
    let dow = if cron_field_is_full(days_of_week, 0, 6) {
        String::new()
    } else {
        let names: Vec<&str> = days_of_week
            .iter()
            .map(|d| SYSTEMD_DOW[*d as usize])
            .collect();
        format!("{} ", names.join(","))
    };
    format!(
        "{dow}*-{}-{} {}:{}:00",
        on_calendar_component(&fields.months, 1, 12),
        on_calendar_component(days_of_month, 1, 31),
        on_calendar_component(&fields.hours, 0, 23),
        on_calendar_component(&fields.minutes, 0, 59),
    )
}

/// The `OnCalendar=` expression(s) equivalent to a parsed cron expression.
///
/// Two lines come back for the DOM/DOW OR case, because a single OnCalendar expression ANDs its
/// day-of-month and day-of-week components and cannot express cron's OR. systemd unions repeated
/// `OnCalendar=` lines in a `[Timer]` section ("the timer triggers whenever any of the expressions
/// elapse"), so `1st OR Monday` becomes `every Monday` + `the 1st of any month` — exactly the OR,
/// with no risk of the silent AND that would fire on the wrong days.
fn cron_to_on_calendar(fields: &CronFields) -> Vec<String> {
    if fields.day_fields_or() {
        vec![
            on_calendar_line(fields, &fields.days_of_month, &ALL_DAYS_OF_WEEK),
            on_calendar_line(fields, &all_days_of_month(), &fields.days_of_week),
        ]
    } else {
        vec![on_calendar_line(
            fields,
            &fields.days_of_month,
            &fields.days_of_week,
        )]
    }
}

// --- launchd ---

/// One `StartCalendarInterval` dict as ordered key/value pairs. Keys we omit are wildcards to
/// launchd, which is how `*` fields are expressed.
type LaunchdInterval = Vec<(&'static str, u32)>;

/// launchd matches an *array* of calendar dicts (any dict firing is enough), but a single dict has
/// no repetition syntax — so a cron expression becomes the cross product of its non-wildcard
/// fields. Bound it: past this many entries the plist is a sign the user wanted `--every`, and a
/// clear error beats a 30k-line agent.
const MAX_LAUNCHD_INTERVALS: usize = 1000;

fn cron_to_launchd_intervals(expr: &str, fields: &CronFields) -> Result<Vec<LaunchdInterval>> {
    let months_full = cron_field_is_full(&fields.months, 1, 12);
    let dom_full = cron_field_is_full(&fields.days_of_month, 1, 31);
    let dow_full = cron_field_is_full(&fields.days_of_week, 0, 6);

    // Whether launchd ANDs or ORs `Day` and `Weekday` inside one dict is not documented and reports
    // disagree, so we never emit a dict containing both — for the OR case by splitting into two
    // groups (below), and here by refusing the AND case outright rather than betting on it.
    if !fields.day_fields_or() && !dom_full && !dow_full {
        anyhow::bail!(
            "cron `{expr}` restricts both day-of-month and day-of-week, and launchd's \
             StartCalendarInterval has no defined behaviour when a single entry sets both Day and \
             Weekday — refusing rather than installing a timer that may fire on the wrong days. \
             Leave one of the two fields as `*`, or use --every/--at."
        );
    }

    // The OR case is expressible: two groups of dicts, one keyed by Day and one by Weekday, unioned
    // by launchd because any matching dict fires.
    let groups: Vec<(Vec<u32>, Vec<u32>)> = if fields.day_fields_or() {
        vec![
            (fields.days_of_month.clone(), ALL_DAYS_OF_WEEK.to_vec()),
            (all_days_of_month(), fields.days_of_week.clone()),
        ]
    } else {
        vec![(fields.days_of_month.clone(), fields.days_of_week.clone())]
    };

    let mut out: Vec<LaunchdInterval> = Vec::new();
    for (dom, dow) in groups {
        let dom_values: Vec<Option<u32>> = if cron_field_is_full(&dom, 1, 31) {
            vec![None]
        } else {
            dom.iter().map(|d| Some(*d)).collect()
        };
        let dow_values: Vec<Option<u32>> = if cron_field_is_full(&dow, 0, 6) {
            vec![None]
        } else {
            dow.iter().map(|d| Some(*d)).collect()
        };
        let month_values: Vec<Option<u32>> = if months_full {
            vec![None]
        } else {
            fields.months.iter().map(|m| Some(*m)).collect()
        };
        let hour_values: Vec<Option<u32>> = if cron_field_is_full(&fields.hours, 0, 23) {
            vec![None]
        } else {
            fields.hours.iter().map(|h| Some(*h)).collect()
        };
        let minute_values: Vec<Option<u32>> = if cron_field_is_full(&fields.minutes, 0, 59) {
            vec![None]
        } else {
            fields.minutes.iter().map(|m| Some(*m)).collect()
        };

        for month in &month_values {
            for day in &dom_values {
                for weekday in &dow_values {
                    for hour in &hour_values {
                        for minute in &minute_values {
                            let mut entry: LaunchdInterval = Vec::new();
                            if let Some(v) = month {
                                entry.push(("Month", *v));
                            }
                            if let Some(v) = day {
                                entry.push(("Day", *v));
                            }
                            if let Some(v) = weekday {
                                entry.push(("Weekday", *v));
                            }
                            if let Some(v) = hour {
                                entry.push(("Hour", *v));
                            }
                            if let Some(v) = minute {
                                entry.push(("Minute", *v));
                            }
                            out.push(entry);
                            if out.len() > MAX_LAUNCHD_INTERVALS {
                                anyhow::bail!(
                                    "cron `{expr}` expands to more than {MAX_LAUNCHD_INTERVALS} \
                                     launchd calendar entries (launchd has no repetition syntax, \
                                     so every matching minute needs its own entry) — use --every \
                                     for a fixed interval, or narrow the hour/minute fields."
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(out)
}

// --- schtasks ---

const SCHTASKS_DOW: [&str; 7] = ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"];
const SCHTASKS_MONTHS: [&str; 12] = [
    "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
];

/// True when `values` is exactly `start, start+step, …` up to `max` — i.e. the field is a plain
/// every-N repeat that a Task Scheduler `/MO` interval can reproduce.
fn is_step_series(values: &[u32], start: u32, max: u32, step: u32) -> bool {
    (start..=max)
        .step_by(step as usize)
        .eq(values.iter().copied())
}

/// Task Scheduler triggers are far coarser than cron: `every N minutes`, `every N hours from a
/// start time`, or daily/weekly/monthly at exactly ONE start time. Anything else is rejected with
/// the specific limitation named — a task that fires on the wrong days is worse than no task.
fn cron_to_schtasks_trigger(expr: &str, fields: &CronFields) -> Result<Vec<String>> {
    let months_full = cron_field_is_full(&fields.months, 1, 12);
    let dom_full = cron_field_is_full(&fields.days_of_month, 1, 31);
    let dow_full = cron_field_is_full(&fields.days_of_week, 0, 6);
    let hours_full = cron_field_is_full(&fields.hours, 0, 23);
    let arg = |parts: &[&str]| parts.iter().map(|s| s.to_string()).collect::<Vec<String>>();

    if fields.day_fields_or() {
        anyhow::bail!(
            "cron `{expr}` fires when day-of-month OR day-of-week matches, and a Windows scheduled \
             task has exactly one trigger schedule — there is no MONTHLY-or-WEEKLY trigger. \
             Register two schedules (one with `*` for day-of-week, one with `*` for day-of-month) \
             instead."
        );
    }

    let calendar_is_daily = months_full && dom_full && dow_full;

    // `*/15 * * * *` and friends → /SC MINUTE /MO 15.
    if calendar_is_daily && hours_full && fields.minutes.len() > 1 {
        let step = fields.minutes[1] - fields.minutes[0];
        if 60 % step == 0 && is_step_series(&fields.minutes, 0, 59, step) {
            return Ok(arg(&["/SC", "MINUTE", "/MO", &step.to_string()]));
        }
    }

    if fields.minutes.len() == 1 {
        let minute = fields.minutes[0];

        // `30 * * * *` → hourly at :30; `0 */6 * * *` → every 6 hours from 00:00.
        if calendar_is_daily && (hours_full || fields.hours.len() > 1) {
            let step = if hours_full {
                1
            } else {
                fields.hours[1] - fields.hours[0]
            };
            let start = fields.hours[0];
            if 24 % step == 0 && start < step && is_step_series(&fields.hours, start, 23, step) {
                return Ok(arg(&[
                    "/SC",
                    "HOURLY",
                    "/MO",
                    &step.to_string(),
                    "/ST",
                    &format!("{start:02}:{minute:02}"),
                ]));
            }
        }

        if fields.hours.len() == 1 {
            let start = format!("{:02}:{minute:02}", fields.hours[0]);
            if calendar_is_daily {
                return Ok(arg(&["/SC", "DAILY", "/ST", &start]));
            }
            if months_full && dom_full {
                let days: Vec<&str> = fields
                    .days_of_week
                    .iter()
                    .map(|d| SCHTASKS_DOW[*d as usize])
                    .collect();
                return Ok(arg(&[
                    "/SC",
                    "WEEKLY",
                    "/D",
                    &days.join(","),
                    "/ST",
                    &start,
                ]));
            }
            if dow_full && !dom_full {
                let days: Vec<String> = fields.days_of_month.iter().map(u32::to_string).collect();
                let mut out = arg(&["/SC", "MONTHLY", "/D", &days.join(",")]);
                if !months_full {
                    let months: Vec<&str> = fields
                        .months
                        .iter()
                        .map(|m| SCHTASKS_MONTHS[*m as usize - 1])
                        .collect();
                    out.push("/M".to_string());
                    out.push(months.join(","));
                }
                out.push("/ST".to_string());
                out.push(start);
                return Ok(out);
            }
            // Only remaining shape: a month filter with every day of the month selected. Task
            // Scheduler's MONTHLY trigger needs specific days (or a week-of-month), so this one is
            // genuinely inexpressible.
            anyhow::bail!(
                "cron `{expr}` runs every day of specific months, which Windows Task Scheduler \
                 cannot express (its MONTHLY trigger needs specific days of the month). Name the \
                 days, e.g. `{} {} 1-31 {} *`, or use --at.",
                fields.minutes[0],
                fields.hours[0],
                fields
                    .months
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(",")
            );
        }
    }

    anyhow::bail!(
        "cron `{expr}` needs {} start times a day, and a Windows scheduled task has a single start \
         time (its only repeating triggers are `every N minutes` and `every N hours`). Use \
         --every, or register one schedule per start time.",
        fields.hours.len() * fields.minutes.len()
    );
}

// ---------------------------------------------------------------------------
// OS timer install/uninstall. The unit/plist/schtasks STRING renderers below this point are pure
// and unit tested; only `install_timer`/`uninstall_timer` (and the per-OS `install_*`/`uninstall_*`
// they dispatch to) shell out to the real scheduler, so `cargo test` never touches this machine's
// systemd/launchd/registry. `cfg!(target_os = ..)` runtime branches (not `#[cfg]` compile gates) so
// every branch still typechecks on this (Linux) build host.
// ---------------------------------------------------------------------------

fn systemd_user_dir() -> Result<std::path::PathBuf> {
    let home =
        forge_config::home_dir().ok_or_else(|| anyhow::anyhow!("cannot resolve home directory"))?;
    Ok(home.join(".config/systemd/user"))
}

fn launchd_agents_dir() -> Result<std::path::PathBuf> {
    let home =
        forge_config::home_dir().ok_or_else(|| anyhow::anyhow!("cannot resolve home directory"))?;
    Ok(home.join("Library/LaunchAgents"))
}

pub(crate) fn install_timer(
    id: &str,
    spec: &ScheduleSpec,
    task: &str,
    cwd: &str,
    mode: Option<&str>,
    model: Option<&str>,
    forge_exe: &str,
) -> Result<()> {
    if cfg!(target_os = "linux") {
        install_systemd(id, spec, task, cwd, mode, model, forge_exe)
    } else if cfg!(target_os = "macos") {
        install_launchd(id, spec, task, cwd, mode, model, forge_exe)
    } else if cfg!(target_os = "windows") {
        install_schtasks(id, spec, task, cwd, mode, model, forge_exe)
    } else {
        anyhow::bail!("forge schedule has no OS-timer backend for this platform")
    }
}

pub(crate) fn uninstall_timer(id: &str) -> Result<()> {
    if cfg!(target_os = "linux") {
        uninstall_systemd(id)
    } else if cfg!(target_os = "macos") {
        uninstall_launchd(id)
    } else if cfg!(target_os = "windows") {
        uninstall_schtasks(id)
    } else {
        Ok(())
    }
}

/// Run `cmd args…` directly (no shell), surfacing a non-zero exit or spawn failure with stderr
/// attached — the one place that actually touches the host's scheduler.
fn run_checked(cmd: &str, args: &[&str]) -> Result<()> {
    let output = std::process::Command::new(cmd)
        .args(args)
        .output()
        .with_context(|| format!("spawning `{cmd}`"))?;
    if !output.status.success() {
        anyhow::bail!(
            "`{cmd} {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

// --- systemd (Linux) ---

fn render_systemd_service(
    id: &str,
    task: &str,
    cwd: &str,
    mode: Option<&str>,
    model: Option<&str>,
    forge_exe: &str,
) -> String {
    let mut exec = format!("{forge_exe} run {}", quote_unit_arg(task));
    if let Some(m) = mode {
        exec.push_str(&format!(" --mode {m}"));
    }
    if let Some(m) = model {
        exec.push_str(&format!(" --model {}", quote_unit_arg(m)));
    }
    format!(
        "[Unit]\nDescription=Forge scheduled task {id}\n\n\
         [Service]\nType=oneshot\nWorkingDirectory={cwd}\nExecStart={exec}\n"
    )
}

fn render_systemd_timer(id: &str, spec: &ScheduleSpec) -> Result<String> {
    let body = match spec {
        ScheduleSpec::Every(d) => {
            let secs = d.as_secs();
            format!("OnActiveSec={secs}s\nOnUnitActiveSec={secs}s")
        }
        ScheduleSpec::Daily { hour, minute } => {
            format!("OnCalendar=*-*-* {hour:02}:{minute:02}:00")
        }
        // A POSIX expression becomes one OnCalendar= line per systemd expression it needs (two for
        // the DOM/DOW OR case, which systemd unions); anything else is the historical verbatim
        // OnCalendar pass-through, so pre-existing rows render exactly as they always did.
        ScheduleSpec::Cron(expr) => match parse_posix_cron(expr) {
            Some(fields) => cron_to_on_calendar(&fields)
                .into_iter()
                .map(|line| format!("OnCalendar={line}"))
                .collect::<Vec<_>>()
                .join("\n"),
            None => format!("OnCalendar={expr}"),
        },
    };
    Ok(format!(
        "[Unit]\nDescription=Forge schedule timer {id}\n\n\
         [Timer]\n{body}\nPersistent=true\n\n\
         [Install]\nWantedBy=timers.target\n"
    ))
}

fn quote_unit_arg(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

#[allow(clippy::too_many_arguments)]
fn install_systemd(
    id: &str,
    spec: &ScheduleSpec,
    task: &str,
    cwd: &str,
    mode: Option<&str>,
    model: Option<&str>,
    forge_exe: &str,
) -> Result<()> {
    let dir = systemd_user_dir()?;
    std::fs::create_dir_all(&dir).context("creating ~/.config/systemd/user")?;
    let service = render_systemd_service(id, task, cwd, mode, model, forge_exe);
    let timer = render_systemd_timer(id, spec)?;
    std::fs::write(dir.join(format!("forge-{id}.service")), service)
        .context("writing systemd service unit")?;
    std::fs::write(dir.join(format!("forge-{id}.timer")), timer)
        .context("writing systemd timer unit")?;

    run_checked("systemctl", &["--user", "daemon-reload"])?;
    if let Err(e) = run_checked(
        "systemctl",
        &["--user", "enable", "--now", &format!("forge-{id}.timer")],
    ) {
        // Leave nothing behind: the caller rolls the DB row back, so the units (and the
        // `timers.target.wants` symlink `enable` writes before `--now` fails) would be orphans.
        let _ = uninstall_systemd(id);
        // The only spec that can render a unit systemd refuses is the OnCalendar pass-through —
        // everything else is generated from input we validated. systemd's own message ("bad unit
        // file setting") does not say which setting, so name it here.
        return Err(match spec {
            ScheduleSpec::Cron(expr) if parse_posix_cron(expr).is_none() => e.context(format!(
                "`{expr}` is not a 5-field cron expression, so it was written to the timer as a \
                 systemd OnCalendar= expression and systemd rejected it — check it with \
                 `systemd-analyze calendar '{expr}'`, or use standard cron (e.g. `0 6 * * 1`)"
            )),
            _ => e,
        });
    }
    Ok(())
}

fn uninstall_systemd(id: &str) -> Result<()> {
    let dir = systemd_user_dir()?;
    let _ = run_checked(
        "systemctl",
        &["--user", "disable", "--now", &format!("forge-{id}.timer")],
    );
    let _ = std::fs::remove_file(dir.join(format!("forge-{id}.service")));
    let _ = std::fs::remove_file(dir.join(format!("forge-{id}.timer")));
    let _ = run_checked("systemctl", &["--user", "daemon-reload"]);
    Ok(())
}

// --- launchd (macOS) ---

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn render_launchd_plist(
    id: &str,
    task: &str,
    cwd: &str,
    mode: Option<&str>,
    model: Option<&str>,
    forge_exe: &str,
    spec: &ScheduleSpec,
) -> Result<String> {
    let mut args = vec![forge_exe.to_string(), "run".to_string(), task.to_string()];
    if let Some(m) = mode {
        args.push("--mode".to_string());
        args.push(m.to_string());
    }
    if let Some(m) = model {
        args.push("--model".to_string());
        args.push(m.to_string());
    }
    let mut args_xml = String::new();
    for a in &args {
        args_xml.push_str(&format!("        <string>{}</string>\n", xml_escape(a)));
    }

    let schedule_xml = match spec {
        ScheduleSpec::Every(d) => format!(
            "    <key>StartInterval</key>\n    <integer>{}</integer>\n",
            d.as_secs()
        ),
        ScheduleSpec::Daily { hour, minute } => format!(
            "    <key>StartCalendarInterval</key>\n    <dict>\n        \
             <key>Hour</key>\n        <integer>{hour}</integer>\n        \
             <key>Minute</key>\n        <integer>{minute}</integer>\n    </dict>\n"
        ),
        // launchd takes an *array* of calendar dicts and fires when any of them matches, so both a
        // translated cron expression and the DOM/DOW OR case fit. An OnCalendar string does not:
        // launchd has no equivalent grammar, so it is rejected exactly as before.
        ScheduleSpec::Cron(expr) => {
            let parsed = parse_posix_cron(expr).ok_or_else(|| {
                anyhow::anyhow!(
                    "`{expr}` isn't a 5-field cron expression, and macOS has no equivalent of \
                     systemd's OnCalendar syntax — use standard cron (e.g. `0 6 * * 1`), --every, \
                     or --at"
                )
            })?;
            let mut xml = String::from("    <key>StartCalendarInterval</key>\n    <array>\n");
            for entry in cron_to_launchd_intervals(expr, &parsed)? {
                xml.push_str("        <dict>\n");
                for (key, value) in entry {
                    xml.push_str(&format!(
                        "            <key>{key}</key>\n            <integer>{value}</integer>\n"
                    ));
                }
                xml.push_str("        </dict>\n");
            }
            xml.push_str("    </array>\n");
            xml
        }
    };

    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(
        "<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
         \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n",
    );
    out.push_str("<plist version=\"1.0\">\n<dict>\n");
    out.push_str(&format!(
        "    <key>Label</key>\n    <string>dev.forge.schedule.{id}</string>\n"
    ));
    out.push_str("    <key>ProgramArguments</key>\n    <array>\n");
    out.push_str(&args_xml);
    out.push_str("    </array>\n");
    out.push_str(&format!(
        "    <key>WorkingDirectory</key>\n    <string>{}</string>\n",
        xml_escape(cwd)
    ));
    out.push_str(&schedule_xml);
    out.push_str("</dict>\n</plist>\n");
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn install_launchd(
    id: &str,
    spec: &ScheduleSpec,
    task: &str,
    cwd: &str,
    mode: Option<&str>,
    model: Option<&str>,
    forge_exe: &str,
) -> Result<()> {
    let dir = launchd_agents_dir()?;
    std::fs::create_dir_all(&dir).context("creating ~/Library/LaunchAgents")?;
    let plist = render_launchd_plist(id, task, cwd, mode, model, forge_exe, spec)?;
    let path = dir.join(format!("dev.forge.schedule.{id}.plist"));
    std::fs::write(&path, plist).context("writing launchd plist")?;
    run_checked("launchctl", &["load", &path.to_string_lossy()])?;
    Ok(())
}

fn uninstall_launchd(id: &str) -> Result<()> {
    let dir = launchd_agents_dir()?;
    let path = dir.join(format!("dev.forge.schedule.{id}.plist"));
    let _ = run_checked("launchctl", &["unload", &path.to_string_lossy()]);
    let _ = std::fs::remove_file(&path);
    Ok(())
}

// --- Task Scheduler (Windows) ---

/// `Command::Run` has no `--cwd` flag, so the task's `/TR` command line itself `cd`s into the
/// working directory before invoking forge (wrapped in `cmd /C` since `cd` is a shell builtin,
/// not something `schtasks` can exec directly).
fn render_schtasks_create_args(
    id: &str,
    task: &str,
    cwd: &str,
    mode: Option<&str>,
    model: Option<&str>,
    forge_exe: &str,
    spec: &ScheduleSpec,
) -> Result<Vec<String>> {
    let mut inner = format!(
        "cd /d {cwd} && \"{forge_exe}\" run \"{}\"",
        task.replace('"', "\\\"")
    );
    if let Some(m) = mode {
        inner.push_str(&format!(" --mode {m}"));
    }
    if let Some(m) = model {
        inner.push_str(&format!(" --model \"{}\"", m.replace('"', "\\\"")));
    }

    let mut args = vec![
        "/Create".to_string(),
        "/TN".to_string(),
        format!("forge-{id}"),
        "/TR".to_string(),
        format!("cmd /C \"{inner}\""),
        "/F".to_string(),
    ];
    match spec {
        ScheduleSpec::Every(d) => {
            let minutes = (d.as_secs() / 60).max(1);
            args.push("/SC".to_string());
            args.push("MINUTE".to_string());
            args.push("/MO".to_string());
            args.push(minutes.to_string());
        }
        ScheduleSpec::Daily { hour, minute } => {
            args.push("/SC".to_string());
            args.push("DAILY".to_string());
            args.push("/ST".to_string());
            args.push(format!("{hour:02}:{minute:02}"));
        }
        // Task Scheduler expresses much less than cron, so the translator rejects (with the reason)
        // rather than approximating; an OnCalendar string has no Windows meaning at all.
        ScheduleSpec::Cron(expr) => {
            let parsed = parse_posix_cron(expr).ok_or_else(|| {
                anyhow::anyhow!(
                    "`{expr}` isn't a 5-field cron expression, and Windows has no equivalent of \
                     systemd's OnCalendar syntax — use standard cron (e.g. `0 6 * * 1`), --every, \
                     or --at"
                )
            })?;
            args.extend(cron_to_schtasks_trigger(expr, &parsed)?);
        }
    }
    Ok(args)
}

#[allow(clippy::too_many_arguments)]
fn install_schtasks(
    id: &str,
    spec: &ScheduleSpec,
    task: &str,
    cwd: &str,
    mode: Option<&str>,
    model: Option<&str>,
    forge_exe: &str,
) -> Result<()> {
    let args = render_schtasks_create_args(id, task, cwd, mode, model, forge_exe, spec)?;
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    run_checked("schtasks", &arg_refs)
}

fn uninstall_schtasks(id: &str) -> Result<()> {
    let _ = run_checked(
        "schtasks",
        &["/Delete", "/TN", &format!("forge-{id}"), "/F"],
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_every_handles_units_and_rejects_garbage() {
        assert_eq!(
            parse_every("30m").unwrap(),
            std::time::Duration::from_secs(1800)
        );
        assert_eq!(
            parse_every("1h").unwrap(),
            std::time::Duration::from_secs(3600)
        );
        assert_eq!(
            parse_every("2d").unwrap(),
            std::time::Duration::from_secs(172_800)
        );
        assert_eq!(
            parse_every("45s").unwrap(),
            std::time::Duration::from_secs(45)
        );
        assert!(parse_every("abc").is_err());
        assert!(parse_every("0m").is_err());
        assert!(parse_every("10x").is_err());
    }

    #[test]
    fn parse_at_handles_valid_and_invalid_times() {
        assert_eq!(parse_at("09:30").unwrap(), (9, 30));
        assert_eq!(parse_at("23:59").unwrap(), (23, 59));
        assert!(parse_at("24:00").is_err());
        assert!(parse_at("9").is_err());
        assert!(parse_at("09:60").is_err());
    }

    #[test]
    fn schedule_spec_stored_round_trips() {
        let every = ScheduleSpec::Every(std::time::Duration::from_secs(1800));
        assert_eq!(every.to_stored(), "every:1800");
        assert_eq!(ScheduleSpec::from_stored("every:1800"), Some(every));

        let daily = ScheduleSpec::Daily { hour: 9, minute: 5 };
        assert_eq!(daily.to_stored(), "daily:09:05");
        assert_eq!(ScheduleSpec::from_stored("daily:09:05"), Some(daily));

        let cron = ScheduleSpec::Cron("Mon *-*-* 09:00:00".to_string());
        assert_eq!(cron.to_stored(), "cron:Mon *-*-* 09:00:00");
        assert_eq!(
            ScheduleSpec::from_stored("cron:Mon *-*-* 09:00:00"),
            Some(cron)
        );

        assert_eq!(ScheduleSpec::from_stored("garbage"), None);
    }

    #[test]
    fn systemd_service_unit_contains_task_cwd_and_exec() {
        let unit = render_systemd_service(
            "abc123",
            "check the deploy",
            "/home/user/proj",
            Some("bypass"),
            Some("openai::gpt-4o"),
            "/usr/local/bin/forge",
        );
        assert!(unit.contains("WorkingDirectory=/home/user/proj"));
        assert!(unit.contains("\"check the deploy\""));
        assert!(unit.contains("/usr/local/bin/forge run"));
        assert!(unit.contains("--mode bypass"));
        assert!(unit.contains("--model \"openai::gpt-4o\""));
    }

    /// The `OnCalendar=` values a spec renders to, one per line.
    fn on_calendar_of(expr: &str) -> Vec<String> {
        render_systemd_timer("abc123", &ScheduleSpec::Cron(expr.into()))
            .unwrap()
            .lines()
            .filter_map(|line| line.strip_prefix("OnCalendar=").map(str::to_string))
            .collect()
    }

    #[test]
    fn posix_cron_translates_to_on_calendar() {
        assert_eq!(on_calendar_of("0 6 * * 1"), ["Mon *-*-* 06:00:00"]);
        assert_eq!(on_calendar_of("0 6 * * MON"), ["Mon *-*-* 06:00:00"]);
        assert_eq!(on_calendar_of("@daily"), ["*-*-* 00:00:00"]);
        assert_eq!(on_calendar_of("* * * * *"), ["*-*-* *:*:00"]);
        assert_eq!(on_calendar_of("*/15 * * * *"), ["*-*-* *:00,15,30,45:00"]);
        assert_eq!(on_calendar_of("0 0 1 * *"), ["*-*-01 00:00:00"]);
        assert_eq!(on_calendar_of("30 2 * JAN,JUL *"), ["*-01,07-* 02:30:00"]);
        assert_eq!(
            on_calendar_of("0 9 * * MON-FRI"),
            ["Mon,Tue,Wed,Thu,Fri *-*-* 09:00:00"]
        );
        // Sunday is 0 or 7; both fold onto the same day.
        assert_eq!(on_calendar_of("0 0 * * 7"), ["Sun *-*-* 00:00:00"]);
        // `*/2` sets Vixie's star flag, so day-of-month still ANDs with day-of-week, but the value
        // set is not `*` and must be enumerated.
        assert_eq!(
            on_calendar_of("0 0 */10 * MON"),
            ["Mon *-*-01,11,21,31 00:00:00"]
        );
    }

    #[test]
    fn cron_dom_dow_or_becomes_two_on_calendar_lines() {
        // POSIX ORs day-of-month with day-of-week when both are restricted; one OnCalendar line
        // would AND them and fire only on Mondays that fall on the 1st.
        assert_eq!(
            on_calendar_of("0 0 1 * MON"),
            ["*-*-01 00:00:00", "Mon *-*-* 00:00:00"]
        );
    }

    #[test]
    fn non_cron_expressions_pass_through_to_on_calendar_verbatim() {
        // Back-compat: `--cron` meant "systemd OnCalendar=" before the translator existed, and rows
        // written then must still render byte-identically.
        for expr in [
            "Mon *-*-* 09:00:00",
            "*-*-* 06:00:00",
            "Mon..Fri *-*-* 09,17:00:00",
            "hourly",
            "weekly",
        ] {
            assert_eq!(on_calendar_of(expr), [expr], "pass-through of `{expr}`");
        }
    }

    #[test]
    fn parse_posix_cron_rejects_what_it_does_not_understand() {
        for expr in [
            "",
            "0 6 * *",         // four fields
            "0 6 * * * *",     // six fields
            "60 0 * * *",      // minute out of range
            "0 24 * * *",      // hour out of range
            "0 0 0 * *",       // day-of-month is 1-based
            "0 0 * 13 *",      // month out of range
            "0 0 * * 8",       // day-of-week is 0-7
            "*/0 * * * *",     // zero step
            "0 0 * * FRI-MON", // reversed range, an error in Vixie cron
            "a b c d e",
            "@reboot",
            "Mon *-*-* 09:00:00",
        ] {
            assert!(
                parse_posix_cron(expr).is_none(),
                "`{expr}` should not parse as POSIX cron"
            );
        }
    }

    #[test]
    fn describe_names_the_dialect_that_was_recognised() {
        assert_eq!(
            ScheduleSpec::Cron("0 6 * * 1".into()).describe(),
            "cron `0 6 * * 1`"
        );
        assert_eq!(
            ScheduleSpec::Cron("Mon *-*-* 06:00:00".into()).describe(),
            "OnCalendar `Mon *-*-* 06:00:00`"
        );
    }

    /// Every expression we generate must actually be accepted by systemd. Skips when
    /// `systemd-analyze` is missing (macOS, Windows, minimal CI images).
    #[test]
    fn generated_on_calendar_expressions_are_valid_systemd() {
        let probe = std::process::Command::new("systemd-analyze")
            .args(["calendar", "*-*-* 00:00:00"])
            .output();
        let Ok(probe) = probe else {
            eprintln!("skipping: systemd-analyze not available");
            return;
        };
        if !probe.status.success() {
            eprintln!("skipping: systemd-analyze cannot parse a known-good expression");
            return;
        }

        for expr in [
            "0 6 * * 1",
            "* * * * *",
            "*/15 * * * *",
            "0 0 1 * *",
            "30 2 * JAN,JUL *",
            "0 9 * * MON-FRI",
            "0 0 1 * MON",
            "15,45 8-17 * * *",
            "@weekly",
        ] {
            for line in on_calendar_of(expr) {
                let out = std::process::Command::new("systemd-analyze")
                    .args(["calendar", &line])
                    .output()
                    .expect("running systemd-analyze");
                assert!(
                    out.status.success(),
                    "systemd rejected `{line}` (from cron `{expr}`): {}",
                    String::from_utf8_lossy(&out.stderr).trim()
                );
            }
        }
    }

    #[test]
    fn systemd_timer_unit_encodes_every_daily_and_cron() {
        let every = render_systemd_timer(
            "abc123",
            &ScheduleSpec::Every(std::time::Duration::from_secs(1800)),
        )
        .unwrap();
        assert!(every.contains("OnUnitActiveSec=1800s"));

        let daily = render_systemd_timer(
            "abc123",
            &ScheduleSpec::Daily {
                hour: 9,
                minute: 30,
            },
        )
        .unwrap();
        assert!(daily.contains("OnCalendar=*-*-* 09:30:00"));

        let cron =
            render_systemd_timer("abc123", &ScheduleSpec::Cron("*-*-* 06:00:00".into())).unwrap();
        assert!(cron.contains("OnCalendar=*-*-* 06:00:00"));
    }

    #[test]
    fn launchd_plist_contains_task_cwd_and_interval() {
        let plist = render_launchd_plist(
            "abc123",
            "check the deploy",
            "/Users/me/proj",
            None,
            None,
            "/usr/local/bin/forge",
            &ScheduleSpec::Every(std::time::Duration::from_secs(3600)),
        )
        .unwrap();
        assert!(plist.contains("<string>check the deploy</string>"));
        assert!(plist.contains("<string>/Users/me/proj</string>"));
        assert!(plist.contains("<key>StartInterval</key>\n    <integer>3600</integer>"));
        assert!(plist.contains("dev.forge.schedule.abc123"));
    }

    fn launchd_plist_of(expr: &str) -> Result<String> {
        render_launchd_plist(
            "abc123",
            "task",
            "/Users/me",
            None,
            None,
            "/usr/local/bin/forge",
            &ScheduleSpec::Cron(expr.into()),
        )
    }

    #[test]
    fn launchd_plist_translates_cron_to_calendar_intervals() {
        let plist = launchd_plist_of("0 6 * * 1").unwrap();
        assert!(plist.contains("<key>StartCalendarInterval</key>"));
        assert!(plist.contains("<key>Weekday</key>\n            <integer>1</integer>"));
        assert!(plist.contains("<key>Hour</key>\n            <integer>6</integer>"));
        assert!(plist.contains("<key>Minute</key>\n            <integer>0</integer>"));
        // Wildcard fields are expressed by leaving the key out entirely.
        assert!(!plist.contains("<key>Day</key>"));
        assert!(!plist.contains("<key>Month</key>"));

        // launchd entries have no repetition syntax, so two minutes × two hours is four dicts.
        let spread = launchd_plist_of("0,30 9,17 * * *").unwrap();
        assert_eq!(spread.matches("        <dict>").count(), 4);
    }

    #[test]
    fn launchd_or_case_splits_into_day_and_weekday_entries() {
        // `1st OR any Monday`: launchd fires when ANY entry matches, so the OR is expressible as
        // one Day-keyed entry plus one Weekday-keyed entry — never both keys in one dict.
        let plist = launchd_plist_of("0 0 1 * MON").unwrap();
        assert_eq!(plist.matches("        <dict>").count(), 2);
        assert!(plist.contains("<key>Day</key>\n            <integer>1</integer>"));
        assert!(plist.contains("<key>Weekday</key>\n            <integer>1</integer>"));
    }

    #[test]
    fn launchd_rejects_and_of_two_restricted_day_fields() {
        // `*/10` sets the star flag, so POSIX ANDs it with the weekday — a combination launchd has
        // no defined behaviour for.
        let err = launchd_plist_of("0 0 */10 * MON").unwrap_err().to_string();
        assert!(err.contains("Day and Weekday"), "{err}");
        assert!(err.contains("`*`"), "{err}");
    }

    #[test]
    fn launchd_still_rejects_on_calendar_expressions() {
        let err = launchd_plist_of("Mon *-*-* 09:00:00")
            .unwrap_err()
            .to_string();
        assert!(err.contains("OnCalendar"), "{err}");
        assert!(err.contains("0 6 * * 1"), "{err}");
    }

    #[test]
    fn schtasks_args_contain_task_cwd_and_interval() {
        let args = render_schtasks_create_args(
            "abc123",
            "check the deploy",
            "C:\\Users\\me\\proj",
            Some("bypass"),
            None,
            "C:\\forge\\forge.exe",
            &ScheduleSpec::Every(std::time::Duration::from_secs(1800)),
        )
        .unwrap();
        let joined = args.join(" ");
        assert!(joined.contains("check the deploy"));
        assert!(joined.contains("C:\\Users\\me\\proj"));
        assert!(joined.contains("/SC MINUTE"));
        assert!(joined.contains("/MO 30"));
        assert!(joined.contains("forge-abc123"));
    }

    fn schtasks_trigger_of(expr: &str) -> Result<String> {
        let args = render_schtasks_create_args(
            "abc123",
            "task",
            "C:\\x",
            None,
            None,
            "forge.exe",
            &ScheduleSpec::Cron(expr.into()),
        )?;
        // Everything from `/SC` on is the trigger; the prefix is the task name and command line.
        let start = args.iter().position(|a| a == "/SC").expect("a /SC trigger");
        Ok(args[start..].join(" "))
    }

    #[test]
    fn schtasks_translates_the_cron_shapes_windows_can_express() {
        assert_eq!(
            schtasks_trigger_of("* * * * *").unwrap(),
            "/SC MINUTE /MO 1"
        );
        assert_eq!(
            schtasks_trigger_of("*/15 * * * *").unwrap(),
            "/SC MINUTE /MO 15"
        );
        assert_eq!(
            schtasks_trigger_of("30 * * * *").unwrap(),
            "/SC HOURLY /MO 1 /ST 00:30"
        );
        assert_eq!(
            schtasks_trigger_of("0 */6 * * *").unwrap(),
            "/SC HOURLY /MO 6 /ST 00:00"
        );
        assert_eq!(
            schtasks_trigger_of("0 2 * * *").unwrap(),
            "/SC DAILY /ST 02:00"
        );
        assert_eq!(
            schtasks_trigger_of("0 6 * * MON,WED").unwrap(),
            "/SC WEEKLY /D MON,WED /ST 06:00"
        );
        assert_eq!(
            schtasks_trigger_of("0 0 1,15 * *").unwrap(),
            "/SC MONTHLY /D 1,15 /ST 00:00"
        );
        assert_eq!(
            schtasks_trigger_of("0 0 1 JAN,JUL *").unwrap(),
            "/SC MONTHLY /D 1 /M JAN,JUL /ST 00:00"
        );
    }

    #[test]
    fn schtasks_rejects_what_it_cannot_express_with_a_reason() {
        // The DOM/DOW OR: a Windows task has exactly one trigger schedule.
        let or = schtasks_trigger_of("0 0 1 * MON").unwrap_err().to_string();
        assert!(or.contains("day-of-month OR day-of-week"), "{or}");
        assert!(or.contains("two schedules"), "{or}");

        // Several start times a day.
        let many = schtasks_trigger_of("0 9,17 * * *").unwrap_err().to_string();
        assert!(many.contains("2 start times a day"), "{many}");

        // Every day of specific months.
        let months = schtasks_trigger_of("0 0 * JAN *").unwrap_err().to_string();
        assert!(
            months.contains("MONTHLY trigger needs specific days"),
            "{months}"
        );

        // OnCalendar strings remain unsupported on Windows, as before.
        let calendar = schtasks_trigger_of("Mon *-*-* 09:00:00")
            .unwrap_err()
            .to_string();
        assert!(calendar.contains("OnCalendar"), "{calendar}");
    }
}
