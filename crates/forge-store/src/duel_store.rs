//! Repository-scoped duel outcomes and routing-learning projections.

use super::*;

impl Store {
    // --- /duel: model arena outcomes + routing-learning boosts (feature: duel) ---

    /// Apply the routing-learning transform shared by the router projection and scoreboard.
    fn duel_boost(wins: i64, total: i64) -> f64 {
        let losses = total - wins;
        ((wins - losses) as f64 * 0.5).clamp(-2.0, 2.0)
    }

    /// Record one `/duel` candidate's outcome (won or lost) for `repo_key` (the canonicalized repo
    /// root). Called once per candidate every time a duel resolves, so a model's full win/loss
    /// history in this repo can be reconstructed and aggregated by [`Store::duel_boosts`].
    pub fn record_duel_outcome(
        &self,
        repo_key: &str,
        model: &str,
        won: bool,
        task: &str,
    ) -> Result<()> {
        self.lock()?.execute(
            "INSERT INTO duel_outcome (id, repo_key, model, won, task) VALUES (?1, ?2, ?3, ?4, ?5)",
            (forge_types::new_id(), repo_key, model, won as i64, task),
        )?;
        Ok(())
    }

    /// Per-model routing boost for `repo_key`, learned from past `/duel` outcomes: `(wins - losses)
    /// as f64 * 0.5`, clamped to `[-2.0, 2.0]` so a long streak can't permanently dominate routing.
    /// Feeds `HeuristicRouter::with_repo_boosts`. Empty when the repo has no duel history.
    pub fn duel_boosts(&self, repo_key: &str) -> Result<std::collections::HashMap<String, f64>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT model, SUM(won), COUNT(*) FROM duel_outcome
             WHERE repo_key = ?1 GROUP BY model",
        )?;
        let rows = stmt
            .query_map([repo_key], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows
            .into_iter()
            .map(|(model, wins, total)| (model, Self::duel_boost(wins, total)))
            .collect())
    }

    /// The per-model win/loss ledger behind [`Store::duel_boosts`], for the scoreboard view:
    /// `(model, wins, losses, boost)`, most-boosted first. Same source (`duel_outcome`) and the
    /// same boost math, so what the scoreboard shows is exactly what routing applies.
    pub fn model_scoreboard(&self, repo_key: &str) -> Result<Vec<(String, i64, i64, f64)>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT model, SUM(won), COUNT(*) FROM duel_outcome
             WHERE repo_key = ?1 GROUP BY model",
        )?;
        let rows = stmt
            .query_map([repo_key], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut out: Vec<(String, i64, i64, f64)> = rows
            .into_iter()
            .map(|(model, wins, total)| {
                let losses = total - wins;
                (model, wins, losses, Self::duel_boost(wins, total))
            })
            .collect();
        out.sort_by(|a, b| b.3.partial_cmp(&a.3).unwrap_or(std::cmp::Ordering::Equal));
        Ok(out)
    }
}
