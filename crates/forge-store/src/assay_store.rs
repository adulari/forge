//! Durable assay runs, ranked findings, and scope history.

use super::*;

impl Store {
    // --- Assay runs + findings (docs/features/analysis-mode.md) ---

    /// Persist an assay run; returns its id. Add findings with [`add_finding`](Self::add_finding).
    pub fn create_assay_run(&self, scope: &str, cost_usd: f64) -> Result<String> {
        let id = forge_types::new_id();
        self.lock()?.execute(
            "INSERT INTO assay_run (id, scope, cost_usd) VALUES (?1, ?2, ?3)",
            (&id, scope, cost_usd),
        )?;
        Ok(id)
    }

    /// Persist one finding under a run.
    pub fn add_finding(&self, run_id: &str, f: &forge_types::Finding) -> Result<()> {
        self.lock()?.execute(
            "INSERT INTO finding (id, run_id, category, severity, confidence, file, line, title,
             rationale, suggested_fix, effort, lens, verified)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            rusqlite::params![
                f.id,
                run_id,
                f.category.as_str(),
                f.severity.as_str(),
                f.confidence.as_str(),
                f.file,
                f.line,
                f.title,
                f.rationale,
                f.suggested_fix,
                f.effort.as_str(),
                f.lens,
                f.verified as i64,
            ],
        )?;
        Ok(())
    }

    /// Findings of a run, ranked (severity, confidence) at read time.
    pub fn load_findings(&self, run_id: &str) -> Result<Vec<forge_types::Finding>> {
        use forge_types::{Confidence, Effort, FindingCategory, Severity};
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            // Actually rank by (severity, confidence) as the doc promises — the query had no ORDER BY,
            // so SQLite returned insertion order and the UI showed the least-important finding first.
            "SELECT id, category, severity, confidence, file, line, title, rationale,
                    suggested_fix, effort, lens, verified
             FROM finding WHERE run_id = ?1
             ORDER BY CASE severity WHEN 'critical' THEN 0 WHEN 'high' THEN 1 WHEN 'medium' THEN 2 ELSE 3 END,
                      CASE confidence WHEN 'high' THEN 0 WHEN 'medium' THEN 1 ELSE 2 END",
        )?;
        let rows = stmt.query_map([run_id], |row| {
            let category: String = row.get(1)?;
            let severity: String = row.get(2)?;
            let confidence: String = row.get(3)?;
            let effort: String = row.get(9)?;
            Ok(forge_types::Finding {
                id: row.get(0)?,
                category: FindingCategory::parse(&category).unwrap_or(FindingCategory::Correctness),
                severity: Severity::parse(&severity).unwrap_or(Severity::Low),
                confidence: Confidence::parse(&confidence).unwrap_or(Confidence::Low),
                file: row.get(4)?,
                line: row.get(5)?,
                title: row.get(6)?,
                rationale: row.get(7)?,
                suggested_fix: row.get(8)?,
                effort: Effort::parse(&effort).unwrap_or(Effort::Small),
                lens: row.get(10)?,
                verified: row.get::<_, i64>(11)? != 0,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// The most recent assay run for `scope`, excluding `exclude_id` (the just-created run).
    /// Returns `None` when this is the first run for this scope.
    pub fn latest_run_for_scope(&self, scope: &str, exclude_id: &str) -> Result<Option<String>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id FROM assay_run WHERE scope = ?1 AND id != ?2
             ORDER BY created_at DESC, rowid DESC LIMIT 1",
        )?;
        let mut rows = stmt.query([scope, exclude_id])?;
        Ok(rows.next()?.map(|r| r.get(0)).transpose()?)
    }

    /// Past assay runs, newest first: `(id, scope, cost_usd, created_at)`.
    pub fn list_assay_runs(&self) -> Result<Vec<(String, String, f64, i64)>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id, scope, cost_usd, created_at FROM assay_run ORDER BY created_at DESC, rowid DESC",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }
}
