//! `forge stats poster` / TUI `/stats`: a lifetime view of work and spend — a 52-week heatmap,
//! streaks and the busiest providers, drawn from the same per-day rows as the app's Activity tab.

use chrono::{Datelike, Duration, NaiveDate};
use forge_store::{streaks, DayUsage};
use std::collections::BTreeMap;

const SHADES: [char; 5] = ['·', '░', '▒', '▓', '█'];

fn human(n: u64) -> String {
    match n {
        n if n >= 1_000_000_000 => format!("{:.1}B", n as f64 / 1e9),
        n if n >= 1_000_000 => format!("{:.1}M", n as f64 / 1e6),
        n if n >= 1_000 => format!("{:.1}K", n as f64 / 1e3),
        n => n.to_string(),
    }
}

/// Bucket 0 is "no activity"; 1-4 are quartiles of the busiest day so one huge day does not wash
/// the rest out.
fn level(tokens: u64, max: u64) -> usize {
    if tokens == 0 || max == 0 {
        return 0;
    }
    (((tokens as f64 / max as f64) * 4.0).ceil() as usize).clamp(1, 4)
}

pub(crate) fn render(
    rows: &[DayUsage],
    today: NaiveDate,
    window_days: u32,
    provider: Option<&str>,
) -> String {
    let rows: Vec<&DayUsage> = rows
        .iter()
        .filter(|r| provider.is_none_or(|p| r.provider == p))
        .collect();
    let mut per_day: BTreeMap<&str, u64> = BTreeMap::new();
    let mut per_provider: BTreeMap<&str, u64> = BTreeMap::new();
    let (mut tokens, mut calls, mut cost) = (0u64, 0u64, 0.0f64);
    for r in &rows {
        *per_day.entry(r.day.as_str()).or_default() += r.tokens();
        *per_provider.entry(r.provider.as_str()).or_default() += r.tokens();
        tokens += r.tokens();
        calls += r.calls;
        cost += r.cost_usd;
    }
    let scope = provider.map_or_else(|| "all accounts".to_string(), str::to_string);
    let mut out = format!("forge activity — last {window_days} days · {scope}\n\n");
    if rows.is_empty() {
        out.push_str("no usage recorded in this window\n");
        return out;
    }

    let weeks = 52i64;
    let monday_this_week =
        today - Duration::days(i64::from(today.weekday().num_days_from_monday()));
    let start = monday_this_week - Duration::weeks(weeks - 1);
    let window_start = today - Duration::days(i64::from(window_days) - 1);
    let max = per_day.values().copied().max().unwrap_or(0);
    for (row, label) in ["Mon", "   ", "Wed", "   ", "Fri", "   ", "Sun"]
        .iter()
        .enumerate()
    {
        out.push_str(label);
        out.push(' ');
        for week in 0..weeks {
            let date = start + Duration::weeks(week) + Duration::days(row as i64);
            let key = date.format("%Y-%m-%d").to_string();
            let active = date <= today && date >= window_start;
            let cell = if !active || date > today {
                ' '
            } else {
                SHADES[level(per_day.get(key.as_str()).copied().unwrap_or(0), max)]
            };
            out.push(cell);
        }
        out.push('\n');
    }
    out.push('\n');

    let active: Vec<String> = per_day.keys().map(|d| (*d).to_string()).collect();
    let s = streaks(&active, &today.format("%Y-%m-%d").to_string());
    out.push_str(&format!(
        "{} active days · current streak {} · longest {}\n",
        s.active_days, s.current_streak, s.longest_streak
    ));
    out.push_str(&format!(
        "{} tokens · {} model calls · ${:.2} est. list price, not a bill\n",
        human(tokens),
        human(calls),
        cost
    ));
    let mut top: Vec<(&str, u64)> = per_provider.into_iter().collect();
    top.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    out.push('\n');
    for (name, t) in top.into_iter().take(5) {
        let bar = "█".repeat(((t as f64 / tokens.max(1) as f64) * 24.0).round() as usize);
        out.push_str(&format!("{name:<18} {bar} {}\n", human(t)));
    }
    out
}

pub(crate) fn run(
    store: &forge_store::Store,
    days: u32,
    provider: Option<&str>,
) -> anyhow::Result<String> {
    let now = chrono::Local::now();
    let offset = i64::from(chrono::Offset::fix(now.offset()).local_minus_utc());
    let since = now.timestamp() - i64::from(days.clamp(1, 400)) * 86_400;
    let rows = store.usage_by_day(since, offset)?;
    Ok(render(&rows, now.date_naive(), days, provider))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(day: &str, provider: &str, tokens: u64) -> DayUsage {
        DayUsage {
            day: day.into(),
            provider: provider.into(),
            input_tokens: tokens,
            output_tokens: 0,
            cost_usd: 1.0,
            calls: 2,
        }
    }

    #[test]
    fn the_poster_shows_the_heatmap_streaks_and_provider_split() {
        let today = NaiveDate::from_ymd_opt(2026, 3, 18).unwrap();
        let rows = vec![
            row("2026-03-16", "claude-cli", 1000),
            row("2026-03-17", "claude-cli", 4000),
            row("2026-03-18", "gemini", 2000),
            row("2025-12-01", "gemini", 500),
        ];
        let text = render(&rows, today, 365, None);
        assert!(
            text.contains("4 active days · current streak 3 · longest 3"),
            "{text}"
        );
        assert!(text.contains('█'), "{text}");
        assert!(text.contains("claude-cli"), "{text}");
        assert!(text.contains("not a bill"), "{text}");
        assert_eq!(
            text.lines()
                .filter(|l| l.starts_with("Mon") || l.starts_with("Sun"))
                .count(),
            2
        );
    }

    #[test]
    fn filtering_to_one_provider_drops_the_others_and_an_empty_window_says_so() {
        let today = NaiveDate::from_ymd_opt(2026, 3, 18).unwrap();
        let rows = vec![row("2026-03-17", "gemini", 10)];
        assert!(render(&rows, today, 30, Some("claude-cli")).contains("no usage recorded"));
        let only = render(&rows, today, 30, Some("gemini"));
        assert!(
            only.contains("1 active days") && !only.contains("claude-cli"),
            "{only}"
        );
    }
}
