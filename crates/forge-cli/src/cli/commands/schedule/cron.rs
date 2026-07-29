//! POSIX cron parsing and native scheduler trigger rendering.

use anyhow::Result;

// The parser below understands the standard 5-field grammar and each OS's
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
pub(crate) fn cron_to_on_calendar(fields: &CronFields) -> Vec<String> {
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

pub(crate) fn cron_to_launchd_intervals(
    expr: &str,
    fields: &CronFields,
) -> Result<Vec<LaunchdInterval>> {
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
pub(crate) fn cron_to_schtasks_trigger(expr: &str, fields: &CronFields) -> Result<Vec<String>> {
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
