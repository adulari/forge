//! Retention maintenance, kept off the session-open path.
//!
//! `create_session` used to run [`Store::prune`] / [`Store::prune_empty`] inline. A cascading
//! delete of a large stale session holds the single WAL writer for as long as it runs, so opening a
//! session could stall for a minute and every other process (Lattice, the daemon) hit
//! "database is locked". The sweep now runs once per process on a background thread, one session
//! per transaction, under a wall-clock budget.

use super::*;
use std::time::{Duration, Instant};

/// Pause before the first sweep so it never competes with the process's own startup reads.
#[cfg(not(test))]
const MAINTENANCE_START_DELAY: Duration = Duration::from_secs(3);

/// Wall-clock cap for one sweep; whatever is left is picked up by the next process.
#[cfg(not(test))]
const MAINTENANCE_BUDGET: Duration = Duration::from_secs(30);

/// Gap between per-session deletes, so other writers get the lock between transactions.
const MAINTENANCE_YIELD: Duration = Duration::from_millis(25);

/// Sessions one sweep may remove (stale / empty), larger than the old per-open cap because the
/// sweep now runs once per process instead of on every `create_session`.
const MAINTENANCE_PRUNE_MAX: usize = 200;
const MAINTENANCE_PRUNE_EMPTY_MAX: usize = 400;

#[cfg(not(test))]
static MAINTENANCE_STARTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// What one [`Store::run_maintenance`] sweep removed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MaintenanceReport {
    pub pruned: usize,
    pub pruned_empty: usize,
}

impl Store {
    /// Delete stale and empty sessions one transaction at a time, yielding between deletes, until
    /// nothing is left to remove, a per-sweep cap is hit, or `budget` elapses.
    pub fn run_maintenance(&self, budget: Duration) -> Result<MaintenanceReport> {
        let deadline = Instant::now() + budget;
        let mut report = MaintenanceReport::default();
        while report.pruned_empty < MAINTENANCE_PRUNE_EMPTY_MAX && Instant::now() < deadline {
            if self.prune_empty(EMPTY_SESSION_HORIZON_SECS, 1)? == 0 {
                break;
            }
            report.pruned_empty += 1;
            std::thread::sleep(MAINTENANCE_YIELD);
        }
        while report.pruned < MAINTENANCE_PRUNE_MAX && Instant::now() < deadline {
            if self.prune(RETENTION_HORIZON_SECS, 1)? == 0 {
                break;
            }
            report.pruned += 1;
            std::thread::sleep(MAINTENANCE_YIELD);
        }
        Ok(report)
    }

    /// Start the retention sweep on a detached thread, at most once per process. A no-op for
    /// in-memory stores and under `cfg(test)`, where tests call [`Store::run_maintenance`]
    /// directly so nothing deletes rows behind their back.
    pub(crate) fn spawn_maintenance_once(&self) {
        #[cfg(not(test))]
        {
            use std::sync::atomic::Ordering;
            if self.db_path.is_none() || MAINTENANCE_STARTED.swap(true, Ordering::SeqCst) {
                return;
            }
            let handle = Store {
                pool: self.pool.clone(),
                reservation_store_id: self.reservation_store_id.clone(),
                db_path: self.db_path.clone(),
            };
            let _ = std::thread::Builder::new()
                .name("forge-store-maintenance".into())
                .spawn(move || {
                    std::thread::sleep(MAINTENANCE_START_DELAY);
                    match handle.run_maintenance(MAINTENANCE_BUDGET) {
                        Ok(r) if r != MaintenanceReport::default() => {
                            tracing::debug!(?r, "store maintenance removed old sessions");
                        }
                        Ok(_) => {}
                        Err(e) => tracing::debug!(error = %e, "store maintenance failed"),
                    }
                });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn backdate(store: &Store, id: &str, days: i64) {
        let past = chrono::Utc::now().timestamp() - days * 24 * 60 * 60;
        store
            .lock()
            .unwrap()
            .execute(
                "UPDATE session SET updated_at = ?1, created_at = ?1 WHERE id = ?2",
                rusqlite::params![past, id],
            )
            .unwrap();
    }

    #[test]
    fn create_session_does_not_prune_and_maintenance_does_in_batches() {
        let store = Store::open_in_memory().unwrap();
        let stale: Vec<String> = (0..3)
            .map(|_| {
                let id = store.create_session("/old", "default").unwrap();
                store.add_message(&id, 0, Role::User, "hi", None).unwrap();
                backdate(&store, &id, 120);
                id
            })
            .collect();

        store.create_session("/new", "default").unwrap();
        for id in &stale {
            assert!(
                store.session_cost(id).is_ok(),
                "create_session must not run the retention sweep"
            );
        }

        let report = store.run_maintenance(Duration::from_secs(10)).unwrap();
        assert_eq!(report.pruned, 3);
        for id in &stale {
            assert!(store.session_cost(id).is_err(), "swept by maintenance");
        }
    }

    #[test]
    fn maintenance_stops_at_the_time_budget() {
        let store = Store::open_in_memory().unwrap();
        let id = store.create_session("/old", "default").unwrap();
        store.add_message(&id, 0, Role::User, "hi", None).unwrap();
        backdate(&store, &id, 120);
        let report = store.run_maintenance(Duration::ZERO).unwrap();
        assert_eq!(report, MaintenanceReport::default());
        assert!(store.session_cost(&id).is_ok());
    }

    /// Every `ON DELETE CASCADE` child column needs a leading index, or each deleted parent row
    /// full-scans the child table (a 1,438-message session took 114 s to delete).
    #[test]
    fn every_cascade_foreign_key_has_a_leading_index() {
        let store = Store::open_in_memory().unwrap();
        let conn = store.lock().unwrap();
        let tables: Vec<String> = conn
            .prepare(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
            )
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let mut missing = Vec::new();
        for table in tables {
            let fks: Vec<String> = conn
                .prepare(&format!(
                    "SELECT \"from\" FROM pragma_foreign_key_list('{table}') WHERE on_delete = 'CASCADE'"
                ))
                .unwrap()
                .query_map([], |r| r.get(0))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap();
            for col in fks {
                let indexed: i64 = conn
                    .query_row(
                        &format!(
                            "SELECT count(*) FROM pragma_index_list('{table}') il \
                             WHERE (SELECT name FROM pragma_index_info(il.name) WHERE seqno = 0) = ?1"
                        ),
                        [&col],
                        |r| r.get(0),
                    )
                    .unwrap();
                let sole_pk: i64 = conn
                    .query_row(
                        &format!(
                            "SELECT count(*) FROM pragma_table_info('{table}') \
                             WHERE name = ?1 AND pk = 1 \
                             AND (SELECT count(*) FROM pragma_table_info('{table}') WHERE pk > 0) = 1"
                        ),
                        [&col],
                        |r| r.get(0),
                    )
                    .unwrap();
                if indexed == 0 && sole_pk == 0 {
                    missing.push(format!("{table}.{col}"));
                }
            }
        }
        assert!(missing.is_empty(), "unindexed cascade FKs: {missing:?}");
    }

    #[test]
    fn migration_37_upgrades_a_v36_store_and_is_idempotent() {
        let store = Store::open_in_memory().unwrap();
        let conn = store.lock().unwrap();
        conn.execute_batch(
            "DROP INDEX idx_tool_call_message; DROP INDEX idx_workflow_run_session;
             DROP INDEX idx_session_cwd; DROP INDEX idx_message_session_role;",
        )
        .unwrap();
        migrations::MIGRATIONS[36](&conn).unwrap();
        migrations::MIGRATIONS[36](&conn).unwrap();
        let plan: String = conn
            .query_row(
                "EXPLAIN QUERY PLAN DELETE FROM tool_call WHERE message_id = 'x'",
                [],
                |r| r.get(3),
            )
            .unwrap();
        assert!(plan.contains("idx_tool_call_message"), "{plan}");
    }
}
