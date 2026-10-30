//! Per-day, per-provider usage totals for the Usage activity heatmap, the TUI `/stats` poster and
//! the mobile Profile card. Read-only aggregation over `usage`; no schema change.

use crate::{Result, Store, USAGE_PROVIDER_EXPR};

/// One local calendar day of one provider's usage.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DayUsage {
    /// `YYYY-MM-DD` in the caller's timezone.
    pub day: String,
    pub provider: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
    /// Model round trips; a row with no recorded count stands for one.
    pub calls: u64,
}

impl DayUsage {
    pub fn tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens
    }
}

/// Totals and streaks over a set of active days.
#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityStreaks {
    pub active_days: u32,
    pub current_streak: u32,
    pub longest_streak: u32,
}

/// `days` are `YYYY-MM-DD` strings of active days (any order, duplicates allowed); `today` is the
/// caller's current local day. A streak that ended yesterday is still current: today is not over.
pub fn streaks(days: &[String], today: &str) -> ActivityStreaks {
    let mut ordinals: Vec<i64> = days.iter().filter_map(|d| day_ordinal(d)).collect();
    ordinals.sort_unstable();
    ordinals.dedup();
    let mut out = ActivityStreaks {
        active_days: u32::try_from(ordinals.len()).unwrap_or(u32::MAX),
        ..ActivityStreaks::default()
    };
    let mut run = 0u32;
    let mut previous: Option<i64> = None;
    for &ordinal in &ordinals {
        run = if previous == Some(ordinal - 1) {
            run + 1
        } else {
            1
        };
        out.longest_streak = out.longest_streak.max(run);
        previous = Some(ordinal);
    }
    if let (Some(last), Some(today)) = (previous, day_ordinal(today)) {
        if last == today || last == today - 1 {
            out.current_streak = run;
        }
    }
    out
}

fn day_ordinal(day: &str) -> Option<i64> {
    let date = chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d").ok()?;
    Some(i64::from(chrono::Datelike::num_days_from_ce(&date)))
}

impl Store {
    /// Usage grouped by local day and provider since `since_epoch`. `utc_offset_secs` shifts the
    /// day boundary to the caller's timezone.
    pub fn usage_by_day(&self, since_epoch: i64, utc_offset_secs: i64) -> Result<Vec<DayUsage>> {
        let conn = self.lock()?;
        let sql = format!(
            "SELECT date(u.created_at + ?2, 'unixepoch') AS day, {USAGE_PROVIDER_EXPR} AS prov, \
             COALESCE(SUM(u.input_tokens), 0), COALESCE(SUM(u.output_tokens), 0), \
             COALESCE(SUM(u.cost_usd), 0.0), COALESCE(SUM(COALESCE(u.api_calls, 1)), 0) \
             FROM usage u JOIN message m ON m.id = u.message_id WHERE u.created_at >= ?1 \
             GROUP BY day, prov ORDER BY day ASC, prov ASC"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([since_epoch, utc_offset_secs], |r| {
            Ok(DayUsage {
                day: r.get(0)?,
                provider: r.get(1)?,
                input_tokens: r.get::<_, i64>(2)? as u64,
                output_tokens: r.get::<_, i64>(3)? as u64,
                cost_usd: r.get(4)?,
                calls: r.get::<_, i64>(5)? as u64,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_types::{Role, Usage};

    fn days(list: &[&str]) -> Vec<String> {
        list.iter().map(|d| (*d).to_string()).collect()
    }

    #[test]
    fn a_streak_ending_yesterday_is_still_current_and_a_gap_resets_it() {
        let s = streaks(
            &days(&[
                "2026-01-01",
                "2026-01-02",
                "2026-01-03",
                "2026-01-10",
                "2026-01-11",
            ]),
            "2026-01-12",
        );
        assert_eq!(s.active_days, 5);
        assert_eq!(s.longest_streak, 3);
        assert_eq!(s.current_streak, 2);
        assert_eq!(
            streaks(&days(&["2026-01-10"]), "2026-01-12").current_streak,
            0
        );
    }

    #[test]
    fn streaks_cross_month_and_leap_boundaries() {
        let s = streaks(
            &days(&["2028-02-28", "2028-02-29", "2028-03-01"]),
            "2028-03-01",
        );
        assert_eq!((s.longest_streak, s.current_streak), (3, 3));
    }

    #[test]
    fn usage_is_bucketed_by_the_callers_local_day_and_provider() {
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/x", "default").unwrap();
        let usage = Usage {
            input_tokens: 100,
            output_tokens: 10,
            cost_usd: 0.5,
            ..Default::default()
        };
        let mut ids = Vec::new();
        for (seq, model) in [
            (0, "gemini::flash"),
            (1, "gemini::flash"),
            (2, "claude-cli::sonnet"),
        ] {
            let m = store
                .add_message(&sid, seq, Role::Assistant, "x", Some(model))
                .unwrap();
            store.record_usage(&sid, &m, &usage, None).unwrap();
            ids.push(m);
        }
        // 2026-03-01 23:30 UTC, 2026-03-02 00:30 UTC, and one a day later.
        let stamps = [1_772_407_800i64, 1_772_411_400, 1_772_497_800];
        for (m, at) in ids.iter().zip(stamps) {
            store
                .lock()
                .unwrap()
                .execute(
                    "UPDATE usage SET created_at = ?1 WHERE message_id = ?2",
                    rusqlite::params![at, m],
                )
                .unwrap();
        }
        let utc = store.usage_by_day(0, 0).unwrap();
        assert_eq!(utc.len(), 3);
        assert_eq!(utc[0].day, "2026-03-01");
        let plus_two = store.usage_by_day(0, 7200).unwrap();
        let gemini: Vec<_> = plus_two.iter().filter(|d| d.provider == "gemini").collect();
        assert_eq!(gemini.len(), 1);
        assert_eq!(gemini[0].day, "2026-03-02");
        assert_eq!((gemini[0].calls, gemini[0].tokens()), (2, 220));
    }
}
