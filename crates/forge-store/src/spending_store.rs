//! Store spending and usage summaries.

use super::*;

impl Store {
    /// Spend across all sessions in the current local calendar day.
    pub fn spend_today_usd(&self) -> Result<f64> {
        let (s, e) = day_bounds_local(chrono::Local::now());
        self.spend_between(s, e)
    }

    /// Spend across all sessions in the current local calendar month.
    pub fn spend_this_month_usd(&self) -> Result<f64> {
        let (s, e) = month_bounds_local(chrono::Local::now());
        self.spend_between(s, e)
    }

    /// Per-model spend + token counts for the current calendar day.
    /// Returns `Vec<(model, cost_usd, input_tokens, output_tokens)>`, sorted by cost desc.
    /// Rows where `message.model` is NULL (side calls like compact/diagnose) are grouped under
    /// the empty string.
    pub fn spend_by_model_today(&self) -> Result<Vec<(String, f64, u64, u64)>> {
        let (s, e) = day_bounds_local(chrono::Local::now());
        let conn = self.lock()?;
        let mut stmt = conn.prepare_cached(
            "SELECT COALESCE(m.model, '') as mdl,
                    COALESCE(SUM(u.cost_usd), 0.0),
                    COALESCE(SUM(u.input_tokens), 0),
                    COALESCE(SUM(u.output_tokens), 0)
             FROM usage u JOIN message m ON m.id = u.message_id
             WHERE u.created_at >= ?1 AND u.created_at < ?2
             GROUP BY mdl
             ORDER BY SUM(u.cost_usd) DESC, SUM(u.input_tokens) DESC",
        )?;
        let rows = stmt.query_map((s, e), |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, f64>(1)?,
                r.get::<_, i64>(2)? as u64,
                r.get::<_, i64>(3)? as u64,
            ))
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Spend in the last 5 hours (rolling, not calendar-day-aligned).
    pub fn spend_last_5h_usd(&self) -> Result<f64> {
        let (s, e) = rolling_hours_bounds(chrono::Local::now(), 5);
        self.spend_between(s, e)
    }

    /// Spend in the current local ISO calendar week (Monday 00:00 → now).
    pub fn spend_this_week_usd(&self) -> Result<f64> {
        let (s, e) = week_bounds_local(chrono::Local::now());
        self.spend_between(s, e)
    }

    /// Today / week / month spend in a single query — 3× cheaper than calling the three
    /// individual helpers. Uses conditional aggregation over the widest window (month) so
    /// only one table scan runs; the `created_at` index makes it sub-millisecond.
    /// Uses prepare_cached so the statement is compiled once per connection, not once per call.
    pub fn spend_summary_usd(&self) -> Result<(f64, f64, f64)> {
        let now = chrono::Local::now();
        let (day_s, day_e) = day_bounds_local(now);
        let (week_s, _) = week_bounds_local(now);
        let (month_s, month_e) = month_bounds_local(now);
        let conn = self.lock()?;
        let mut stmt = conn.prepare_cached(
            "SELECT
               COALESCE(SUM(CASE WHEN created_at >= ?1 AND created_at < ?2 THEN cost_usd ELSE 0 END), 0.0),
               COALESCE(SUM(CASE WHEN created_at >= ?3 THEN cost_usd ELSE 0 END), 0.0),
               COALESCE(SUM(cost_usd), 0.0)
             FROM usage
             WHERE created_at >= ?4 AND created_at < ?5",
        )?;
        Ok(
            stmt.query_row((day_s, day_e, week_s, month_s, month_e), |row| {
                Ok((
                    row.get::<_, f64>(0)?,
                    row.get::<_, f64>(1)?,
                    row.get::<_, f64>(2)?,
                ))
            })?,
        )
    }

    /// Per-model spend + token counts for the last 5 hours.
    pub fn spend_by_model_5h(&self) -> Result<Vec<(String, f64, u64, u64)>> {
        let (s, e) = rolling_hours_bounds(chrono::Local::now(), 5);
        let conn = self.lock()?;
        let mut stmt = conn.prepare_cached(
            "SELECT COALESCE(m.model, '') as mdl,
                    COALESCE(SUM(u.cost_usd), 0.0),
                    COALESCE(SUM(u.input_tokens), 0),
                    COALESCE(SUM(u.output_tokens), 0)
             FROM usage u JOIN message m ON m.id = u.message_id
             WHERE u.created_at >= ?1 AND u.created_at < ?2
             GROUP BY mdl
             ORDER BY SUM(u.cost_usd) DESC, SUM(u.input_tokens) DESC",
        )?;
        let rows = stmt.query_map((s, e), |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, f64>(1)?,
                r.get::<_, i64>(2)? as u64,
                r.get::<_, i64>(3)? as u64,
            ))
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Per-model spend + token counts for the current ISO week.
    pub fn spend_by_model_week(&self) -> Result<Vec<(String, f64, u64, u64)>> {
        let (s, e) = week_bounds_local(chrono::Local::now());
        let conn = self.lock()?;
        let mut stmt = conn.prepare_cached(
            "SELECT COALESCE(m.model, '') as mdl,
                    COALESCE(SUM(u.cost_usd), 0.0),
                    COALESCE(SUM(u.input_tokens), 0),
                    COALESCE(SUM(u.output_tokens), 0)
             FROM usage u JOIN message m ON m.id = u.message_id
             WHERE u.created_at >= ?1 AND u.created_at < ?2
             GROUP BY mdl
             ORDER BY SUM(u.cost_usd) DESC, SUM(u.input_tokens) DESC",
        )?;
        let rows = stmt.query_map((s, e), |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, f64>(1)?,
                r.get::<_, i64>(2)? as u64,
                r.get::<_, i64>(3)? as u64,
            ))
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Per-model spend + token counts for the current calendar month.
    pub fn spend_by_model_month(&self) -> Result<Vec<(String, f64, u64, u64)>> {
        let (s, e) = month_bounds_local(chrono::Local::now());
        let conn = self.lock()?;
        let mut stmt = conn.prepare_cached(
            "SELECT COALESCE(m.model, '') as mdl,
                    COALESCE(SUM(u.cost_usd), 0.0),
                    COALESCE(SUM(u.input_tokens), 0),
                    COALESCE(SUM(u.output_tokens), 0)
             FROM usage u JOIN message m ON m.id = u.message_id
             WHERE u.created_at >= ?1 AND u.created_at < ?2
             GROUP BY mdl
             ORDER BY SUM(u.cost_usd) DESC, SUM(u.input_tokens) DESC",
        )?;
        let rows = stmt.query_map((s, e), |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, f64>(1)?,
                r.get::<_, i64>(2)? as u64,
                r.get::<_, i64>(3)? as u64,
            ))
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}
