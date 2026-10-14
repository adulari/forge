//! Scored benchmark runs per model (migration #43), the measurements behind the mesh's
//! cost-per-solve signal.

use super::*;

/// One scored run of one model: a metrics sidecar joined with its own evaluation.
#[derive(Debug, Clone, PartialEq)]
pub struct BenchSolveRun {
    /// The metrics sidecar the run came from; recording it again replaces the row.
    pub source: String,
    /// Catalog id (`provider::model`).
    pub model: String,
    /// pass@1-eligible attempts.
    pub attempts: u32,
    pub solved: u32,
    /// Notional spend over those attempts.
    pub cost_usd: f64,
}

/// Every recorded run of one model, pooled.
#[derive(Debug, Clone, PartialEq)]
pub struct BenchSolveTotals {
    pub model: String,
    pub runs: u32,
    pub attempts: u32,
    pub solved: u32,
    pub cost_usd: f64,
}

impl Store {
    /// Insert or replace scored runs, keyed by source, in one transaction.
    pub fn record_bench_solves(&self, runs: &[BenchSolveRun]) -> Result<()> {
        with_busy_retry(|| {
            let mut conn = self.lock()?;
            let tx = conn.transaction()?;
            for run in runs {
                tx.execute(
                    "INSERT INTO bench_solve (source, model, attempts, solved, cost_usd, recorded_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, strftime('%s','now'))
                     ON CONFLICT(source) DO UPDATE SET
                       model = excluded.model, attempts = excluded.attempts,
                       solved = excluded.solved, cost_usd = excluded.cost_usd,
                       recorded_at = excluded.recorded_at",
                    rusqlite::params![
                        run.source,
                        run.model,
                        run.attempts,
                        run.solved,
                        run.cost_usd
                    ],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
    }

    /// Per-model totals over every recorded run, by model id.
    pub fn bench_solve_totals(&self) -> Result<Vec<BenchSolveTotals>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare_cached(
            "SELECT model, COUNT(*), SUM(attempts), SUM(solved), SUM(cost_usd)
             FROM bench_solve GROUP BY model ORDER BY model",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(BenchSolveTotals {
                    model: r.get(0)?,
                    runs: r.get(1)?,
                    attempts: r.get(2)?,
                    solved: r.get(3)?,
                    cost_usd: r.get(4)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str, model: &str, attempts: u32, solved: u32, cost: f64) -> BenchSolveRun {
        BenchSolveRun {
            source: source.into(),
            model: model.into(),
            attempts,
            solved,
            cost_usd: cost,
        }
    }

    #[test]
    fn runs_pool_per_model_and_a_re_record_replaces_instead_of_double_counting() {
        let store = Store::open_in_memory().unwrap();
        store
            .record_bench_solves(&[
                run("a.metrics.jsonl", "openai::gpt-5.5", 10, 4, 1.0),
                run("b.metrics.jsonl", "openai::gpt-5.5", 12, 6, 1.5),
                run("c.metrics.jsonl", "groq::x", 20, 2, 0.2),
            ])
            .unwrap();
        store
            .record_bench_solves(&[run("a.metrics.jsonl", "openai::gpt-5.5", 10, 5, 1.0)])
            .unwrap();
        let totals = store.bench_solve_totals().unwrap();
        assert_eq!(
            totals,
            vec![
                BenchSolveTotals {
                    model: "groq::x".into(),
                    runs: 1,
                    attempts: 20,
                    solved: 2,
                    cost_usd: 0.2,
                },
                BenchSolveTotals {
                    model: "openai::gpt-5.5".into(),
                    runs: 2,
                    attempts: 22,
                    solved: 11,
                    cost_usd: 2.5,
                },
            ]
        );
    }
}
