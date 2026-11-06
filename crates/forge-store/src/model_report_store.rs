//! Per-model report card: how each model actually did on this machine over a trailing window.
//!
//! Counts and medians only. No prompt, reply or tool-argument text is ever read: the queries
//! touch `mesh_outcome` (one row per routed model call), `usage` joined to `message` for cost,
//! and `tool_call.status` joined to `message.model` for tool failures.

use std::collections::{BTreeMap, HashMap};

use super::*;

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ModelReportRow {
    pub model: String,
    /// Routed model calls in the window (one `mesh_outcome` row each).
    pub calls: u32,
    /// Calls that returned an answer.
    pub completed: u32,
    /// Failed calls after which the mesh moved the same session to a later failover hop.
    pub failed_over: u32,
    /// Failed calls with no later failover hop in the same session.
    pub failed: u32,
    pub median_latency_ms: Option<i64>,
    /// Median cost of one recorded provider call. Subscription and free routes record $0.
    pub median_cost_usd: Option<f64>,
    pub cost_samples: u32,
    pub tool_calls: u32,
    pub tool_errors: u32,
}

impl ModelReportRow {
    pub fn completion_rate(&self) -> Option<f64> {
        (self.calls > 0).then(|| f64::from(self.completed) / f64::from(self.calls))
    }

    pub fn tool_error_rate(&self) -> Option<f64> {
        (self.tool_calls > 0).then(|| f64::from(self.tool_errors) / f64::from(self.tool_calls))
    }
}

fn median<T: Copy + PartialOrd>(values: &mut [T]) -> Option<T> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Some(values[values.len() / 2])
}

impl Store {
    /// Report card per model for activity at or after `since` (epoch seconds), optionally limited
    /// to sessions whose `cwd` or worktree is exactly `repo`. Rows are ordered by call count.
    pub fn model_report(&self, since: i64, repo: Option<&str>) -> Result<Vec<ModelReportRow>> {
        use rusqlite::types::Value;
        let conn = self.lock()?;
        let mut rows: BTreeMap<String, ModelReportRow> = BTreeMap::new();
        let mut latencies: HashMap<String, Vec<i64>> = HashMap::new();
        let mut costs: HashMap<String, Vec<f64>> = HashMap::new();

        let repo_join = |alias: &str| {
            if repo.is_some() {
                format!(
                    "JOIN session s ON s.id = {alias}.session_id \
                     AND (s.cwd = ?2 OR s.worktree_path = ?2)"
                )
            } else {
                String::new()
            }
        };
        // `?2` is the repo; without one it is bound to NULL so numbered params stay contiguous.
        let base_params = || -> Vec<Value> {
            vec![
                Value::Integer(since),
                repo.map_or(Value::Null, |r| Value::Text(r.to_string())),
            ]
        };

        {
            let sql = format!(
                "SELECT o.session_id, o.model, o.outcome, o.latency_ms, o.failover_hop
                 FROM mesh_outcome o {}
                 WHERE o.completed_at >= ?1 AND (?2 IS NULL OR ?2 IS NOT NULL)
                 ORDER BY o.session_id, o.started_at, o.id",
                repo_join("o")
            );
            let mut stmt = conn.prepare(&sql)?;
            let calls = stmt
                .query_map(rusqlite::params_from_iter(base_params()), |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, i64>(4)?,
                    ))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            // A failure is "failed over" when the same session later logged a higher failover hop:
            // the mesh moved on to another model after it. Rows arrive grouped by session.
            let mut max_hop_after = vec![i64::MIN; calls.len()];
            let mut running = i64::MIN;
            for i in (0..calls.len()).rev() {
                if i + 1 < calls.len() && calls[i + 1].0 != calls[i].0 {
                    running = i64::MIN;
                }
                max_hop_after[i] = running;
                running = running.max(calls[i].4);
            }
            for (i, (_, model, outcome, latency_ms, hop)) in calls.iter().enumerate() {
                let row = rows.entry(model.clone()).or_default();
                row.model.clone_from(model);
                row.calls += 1;
                latencies
                    .entry(model.clone())
                    .or_default()
                    .push(*latency_ms);
                if outcome == "success" {
                    row.completed += 1;
                } else if max_hop_after[i] > *hop {
                    row.failed_over += 1;
                } else {
                    row.failed += 1;
                }
            }
        }

        {
            let sql = format!(
                "SELECT COALESCE(u.model, m.model), u.cost_usd
                 FROM usage u JOIN message m ON m.id = u.message_id {}
                 WHERE u.created_at >= ?1 AND (?2 IS NULL OR ?2 IS NOT NULL)",
                repo_join("m")
            );
            let mut stmt = conn.prepare(&sql)?;
            let usage = stmt
                .query_map(rusqlite::params_from_iter(base_params()), |r| {
                    Ok((r.get::<_, Option<String>>(0)?, r.get::<_, f64>(1)?))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            for (model, cost) in usage {
                if let Some(model) = model {
                    costs.entry(model).or_default().push(cost);
                }
            }
        }

        {
            let sql = format!(
                "SELECT m.model, COUNT(*),
                        SUM(CASE WHEN t.status = 'error' AND t.permission <> 'denied' THEN 1 ELSE 0 END)
                 FROM tool_call t JOIN message m ON m.id = t.message_id {}
                 WHERE t.created_at >= ?1 AND (?2 IS NULL OR ?2 IS NOT NULL)
                   AND m.model IS NOT NULL
                 GROUP BY m.model",
                repo_join("m")
            );
            let mut stmt = conn.prepare(&sql)?;
            let tools = stmt
                .query_map(rusqlite::params_from_iter(base_params()), |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, i64>(2)?,
                    ))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            for (model, total, errors) in tools {
                let row = rows.entry(model.clone()).or_default();
                row.model = model;
                row.tool_calls = total as u32;
                row.tool_errors = errors as u32;
            }
        }

        for (model, row) in &mut rows {
            row.median_latency_ms = latencies.get_mut(model).and_then(|v| median(v));
            if let Some(cost) = costs.get_mut(model) {
                row.cost_samples = cost.len() as u32;
                row.median_cost_usd = median(cost);
            }
        }
        // A model with spend but no routed-call rows (older stores) still gets a card.
        for (model, cost) in &mut costs {
            if !rows.contains_key(model) {
                rows.insert(
                    model.clone(),
                    ModelReportRow {
                        model: model.clone(),
                        cost_samples: cost.len() as u32,
                        median_cost_usd: median(cost),
                        ..ModelReportRow::default()
                    },
                );
            }
        }
        let mut out: Vec<_> = rows.into_values().collect();
        out.sort_by(|a, b| b.calls.cmp(&a.calls).then_with(|| a.model.cmp(&b.model)));
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(store: &Store, session: &str, model: &str, at: i64, ok: bool, hop: u32, ms: u64) {
        store
            .record_mesh_outcome(&MeshOutcome {
                session_id: session.into(),
                model: model.into(),
                tier: TaskTier::Standard,
                started_at: at,
                completed_at: at,
                latency_ms: ms,
                outcome: if ok { "success" } else { "failure" }.into(),
                error_kind: (!ok).then(|| "transient".to_string()),
                failover_hop: hop,
                tool_calls: 0,
                verified_completion: ok,
            })
            .unwrap();
    }

    fn seed() -> (Store, String, String) {
        let store = Store::open_in_memory().unwrap();
        let repo_a = store.create_session("/work/a", "auto").unwrap();
        let repo_b = store.create_session("/work/b", "auto").unwrap();
        (store, repo_a, repo_b)
    }

    #[test]
    fn counts_failures_failovers_and_medians_per_model() {
        let (store, a, _) = seed();
        for n in 0..4 {
            outcome(
                &store,
                &a,
                "x::steady",
                1000 + n,
                true,
                0,
                100 + n as u64 * 100,
            );
        }
        let flaky_session = store.create_session("/work/a", "auto").unwrap();
        outcome(&store, &flaky_session, "x::flaky", 1100, true, 0, 50);
        for n in 0..6 {
            outcome(&store, &flaky_session, "x::flaky", 1200 + n, false, 0, 10);
        }
        outcome(&store, &flaky_session, "x::flaky", 1300, false, 0, 10);
        outcome(&store, &flaky_session, "x::backup", 1301, true, 1, 70);
        let rows = store.model_report(0, None).unwrap();
        let by = |m: &str| rows.iter().find(|r| r.model == m).unwrap().clone();
        let steady = by("x::steady");
        assert_eq!((steady.calls, steady.completed), (4, 4));
        assert_eq!(steady.median_latency_ms, Some(300));
        assert_eq!(steady.completion_rate(), Some(1.0));
        let flaky = by("x::flaky");
        assert_eq!((flaky.calls, flaky.completed), (8, 1));
        assert_eq!(flaky.failed_over, 7);
        assert_eq!(flaky.failed, 0);
        assert!(flaky.completion_rate().unwrap() < 0.2);
        assert_eq!(by("x::backup").completed, 1);
    }

    #[test]
    fn a_failure_with_no_later_hop_is_a_plain_failure() {
        let (store, a, _) = seed();
        outcome(&store, &a, "x::m", 1000, false, 0, 5);
        let row = &store.model_report(0, None).unwrap()[0];
        assert_eq!((row.failed, row.failed_over), (1, 0));
    }

    #[test]
    fn window_and_repo_filters_bound_the_rows() {
        let (store, a, b) = seed();
        outcome(&store, &a, "x::m", 100, true, 0, 5);
        outcome(&store, &a, "x::m", 5000, true, 0, 5);
        outcome(&store, &b, "x::m", 5001, false, 0, 5);
        assert_eq!(store.model_report(1000, None).unwrap()[0].calls, 2);
        let only_a = store.model_report(1000, Some("/work/a")).unwrap();
        assert_eq!((only_a[0].calls, only_a[0].completed), (1, 1));
        assert!(store
            .model_report(1000, Some("/work/none"))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn cost_and_tool_errors_join_through_message_model() {
        let (store, a, _) = seed();
        let m = store
            .add_message(&a, 1, Role::Assistant, "SECRET-TEXT", Some("x::m"))
            .unwrap();
        {
            let conn = store.lock().unwrap();
            for (i, cost) in [0.01, 0.03, 0.02].iter().enumerate() {
                conn.execute(
                    "INSERT INTO usage (id, message_id, model, input_tokens, output_tokens, cost_usd, created_at)
                     VALUES (?1, ?2, 'x::m', 1, 1, ?3, 2000)",
                    rusqlite::params![format!("u{i}"), m, cost],
                )
                .unwrap();
            }
        }
        store
            .record_tool_call(&m, "shell", "{}", "ok", "auto", "ok")
            .unwrap();
        store
            .record_tool_call(&m, "shell", "{}", "bad", "auto", "error")
            .unwrap();
        store
            .record_tool_call(&m, "shell", "{}", "no", "denied", "error")
            .unwrap();
        store
            .lock()
            .unwrap()
            .execute("UPDATE tool_call SET created_at = 2000", [])
            .unwrap();
        let row = &store.model_report(1000, None).unwrap()[0];
        assert_eq!(row.median_cost_usd, Some(0.02));
        assert_eq!((row.tool_calls, row.tool_errors), (3, 1));
        assert_eq!(row.tool_error_rate(), Some(1.0 / 3.0));
        assert!(!serde_json::to_string(row).unwrap().contains("SECRET"));
    }

    #[test]
    fn a_model_with_no_data_is_absent_and_its_rates_are_none() {
        let (store, _, _) = seed();
        assert!(store.model_report(0, None).unwrap().is_empty());
        let empty = ModelReportRow::default();
        assert_eq!(empty.completion_rate(), None);
        assert_eq!(empty.tool_error_rate(), None);
    }

    #[test]
    fn the_window_start_is_found_in_a_large_tool_call_table() {
        let (store, a, _) = seed();
        let m = store
            .add_message(&a, 1, Role::Assistant, "x", Some("x::m"))
            .unwrap();
        {
            let mut conn = store.lock().unwrap();
            let tx = conn.transaction().unwrap();
            for i in 0..5000i64 {
                tx.execute(
                    "INSERT INTO tool_call (id, message_id, tool_name, args_json, permission, status, created_at)
                     VALUES (?1, ?2, 't', '{}', 'auto', 'ok', ?3)",
                    rusqlite::params![format!("t{i}"), m, i],
                )
                .unwrap();
            }
            tx.commit().unwrap();
        }
        let row = &store.model_report(4000, None).unwrap()[0];
        assert_eq!(row.tool_calls, 1000);
    }

    #[test]
    fn report_queries_use_covering_indexes_and_never_scan_result_json() {
        let (store, _, _) = seed();
        let conn = store.lock().unwrap();
        let plan = |sql: &str| -> String {
            conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
                .unwrap()
                .query_map([], |r| r.get::<_, String>(3))
                .unwrap()
                .map(|r| r.unwrap())
                .collect::<Vec<_>>()
                .join(" | ")
        };
        let tools = plan(
            "SELECT m.model, COUNT(*), SUM(CASE WHEN t.status = 'error' AND t.permission <> 'denied' THEN 1 ELSE 0 END)
             FROM tool_call t JOIN message m ON m.id = t.message_id
             WHERE t.created_at >= 1 AND m.model IS NOT NULL GROUP BY m.model",
        );
        assert!(
            tools.contains("COVERING INDEX idx_tool_call_report"),
            "tool_call must be read through its covering index: {tools}"
        );
        let outcomes = plan("SELECT o.model FROM mesh_outcome o WHERE o.completed_at >= 1");
        assert!(
            outcomes.contains("idx_mesh_outcome_completed"),
            "mesh_outcome must be range-scanned on completed_at: {outcomes}"
        );
    }

    /// Generated-store measurement, run by hand: `FORGE_REPORT_BENCH_DIR=<dir> [FORGE_REPORT_BENCH_REUSE=1] cargo test -p
    /// forge-agent-store --release -- --ignored report_bench --nocapture`. ~200k tool_call rows
    /// with 4 KB results spread over 60 days, so half fall in the 30-day window.
    #[test]
    #[ignore]
    fn report_bench() {
        let dir = std::env::var("FORGE_REPORT_BENCH_DIR").expect("FORGE_REPORT_BENCH_DIR");
        let path = std::path::Path::new(&dir).join("report-bench.db");
        let reuse = std::env::var("FORGE_REPORT_BENCH_REUSE").is_ok() && path.exists();
        if !reuse {
            let _ = std::fs::remove_file(&path);
        }
        let opened = std::time::Instant::now();
        let store = Store::open(&path).unwrap();
        println!("open (runs pending migrations): {:?}", opened.elapsed());
        let now = 1_800_000_000i64;
        if !reuse {
            let sid = store.create_session("/work/a", "auto").unwrap();
            let conn = store.lock().unwrap();
            conn.execute_batch(&format!(
                "BEGIN;
                 WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM n WHERE i < 3999)
                 INSERT INTO message (id, session_id, seq, role, content, model, created_at)
                 SELECT 'm' || i, '{sid}', i, 'assistant', 'x', 'x::model' || (i % 5), {now} - (i * 1300)
                 FROM n;
                 WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM n WHERE i < 199999)
                 INSERT INTO tool_call (id, message_id, tool_name, args_json, result_json, permission, status, created_at)
                 SELECT 't' || i, 'm' || (i % 4000), 'shell', '{{}}', hex(randomblob(2048)), 'auto',
                        CASE WHEN i % 17 = 0 THEN 'error' ELSE 'ok' END,
                        {now} - 5_184_000 + (i * 26)
                 FROM n;
                 WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM n WHERE i < 49999)
                 INSERT INTO mesh_outcome (session_id, model, tier, started_at, completed_at, latency_ms, outcome, failover_hop)
                 SELECT '{sid}', 'x::model' || (i % 5), 'standard', {now} - (i * 100) - 5, {now} - (i * 100), 900, 'success', 0
                 FROM n;
                 COMMIT;"
            ))
            .unwrap();
        }
        let since = now - 30 * 86_400;
        let db_mb = std::fs::metadata(&path).unwrap().len() / 1_000_000;
        {
            let conn = store.lock().unwrap();
            for sql in [
                "SELECT m.model, COUNT(*), SUM(CASE WHEN t.status = 'error' AND t.permission <> 'denied' THEN 1 ELSE 0 END) FROM tool_call t JOIN message m ON m.id = t.message_id WHERE t.created_at >= 1 AND m.model IS NOT NULL GROUP BY m.model",
                "SELECT o.model FROM mesh_outcome o WHERE o.completed_at >= 1",
            ] {
                let plan: Vec<String> = conn
                    .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
                    .unwrap()
                    .query_map([], |r| r.get::<_, String>(3))
                    .unwrap()
                    .map(|r| r.unwrap())
                    .collect();
                println!("PLAN {plan:?}");
            }
        }
        for run in 0..3 {
            let t = std::time::Instant::now();
            let rows = store.model_report(since, None).unwrap();
            println!(
                "db {db_mb} MB run {run}: model_report {:?} ({} models, {} tool calls)",
                t.elapsed(),
                rows.len(),
                rows.iter().map(|r| r.tool_calls).sum::<u32>()
            );
        }
    }
}
