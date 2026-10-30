//! Run history for `forge schedule` ticks (docs/features/schedule.md): one row per attempt, so the
//! app can show what actually happened instead of inferring it from a `last_run` timestamp.

use crate::{Result, Schedule, ScheduleRun, Store};

const RUN_COLUMNS: &str = "id, schedule_id, started_at, finished_at, status, trigger, attempt, \
                           session_id, pid, summary, error, read";

fn run_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<ScheduleRun> {
    Ok(ScheduleRun {
        id: r.get(0)?,
        schedule_id: r.get(1)?,
        started_at: r.get(2)?,
        finished_at: r.get(3)?,
        status: r.get(4)?,
        trigger: r.get(5)?,
        attempt: r.get(6)?,
        session_id: r.get(7)?,
        pid: r.get(8)?,
        summary: r.get(9)?,
        error: r.get(10)?,
        read: r.get::<_, i64>(11)? != 0,
    })
}

impl Store {
    /// Open a run in `running` state. A `skipped` row is written with [`Self::finish_schedule_run`]
    /// straight away by the caller.
    #[allow(clippy::too_many_arguments)]
    pub fn begin_schedule_run(
        &self,
        id: &str,
        schedule_id: &str,
        trigger: &str,
        attempt: i64,
        session_id: Option<&str>,
        pid: Option<i64>,
        now: i64,
    ) -> Result<()> {
        self.lock()?.execute(
            "INSERT INTO schedule_run (id, schedule_id, started_at, status, trigger, attempt, \
             session_id, pid) VALUES (?1, ?2, ?3, 'running', ?4, ?5, ?6, ?7)",
            rusqlite::params![id, schedule_id, now, trigger, attempt, session_id, pid],
        )?;
        Ok(())
    }

    /// Close a run with its final status. Also stamps the schedule's `last_run` so the plain
    /// `forge schedule list` column stays truthful.
    pub fn finish_schedule_run(
        &self,
        id: &str,
        status: &str,
        summary: Option<&str>,
        error: Option<&str>,
        now: i64,
    ) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "UPDATE schedule_run SET status = ?1, summary = ?2, error = ?3, finished_at = ?4 \
             WHERE id = ?5",
            rusqlite::params![status, summary, error, now, id],
        )?;
        if status != "skipped" {
            conn.execute(
                "UPDATE schedule SET last_run = ?1 WHERE id = (SELECT schedule_id FROM schedule_run \
                 WHERE id = ?2)",
                rusqlite::params![now, id],
            )?;
        }
        Ok(())
    }

    /// Newest first. `schedule_id` narrows to one schedule; `limit` caps the page.
    pub fn list_schedule_runs(
        &self,
        schedule_id: Option<&str>,
        limit: i64,
    ) -> Result<Vec<ScheduleRun>> {
        let conn = self.lock()?;
        let sql = format!(
            "SELECT {RUN_COLUMNS} FROM schedule_run \
             WHERE (?1 IS NULL OR schedule_id = ?1) ORDER BY started_at DESC, rowid DESC LIMIT ?2"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(rusqlite::params![schedule_id, limit], run_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Ticks of this schedule that ran to an end (first attempts only) - what `max_iterations` counts.
    pub fn count_finished_schedule_runs(&self, schedule_id: &str) -> Result<i64> {
        Ok(self.lock()?.query_row(
            "SELECT COUNT(*) FROM schedule_run WHERE schedule_id = ?1 AND attempt = 1 \
             AND status IN ('ok','failed','interrupted')",
            [schedule_id],
            |r| r.get(0),
        )?)
    }

    /// The `running` rows of a schedule, so a new tick can tell whether the last one is alive.
    pub fn running_schedule_runs(&self, schedule_id: &str) -> Result<Vec<ScheduleRun>> {
        let conn = self.lock()?;
        let sql = format!(
            "SELECT {RUN_COLUMNS} FROM schedule_run WHERE schedule_id = ?1 AND status = 'running'"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map([schedule_id], run_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Mark `running` rows whose process is gone as `interrupted`. `alive` is asked about each
    /// recorded pid; a row with no pid is treated as dead. Returns how many rows changed.
    pub fn interrupt_dead_schedule_runs(
        &self,
        alive: impl Fn(i64) -> bool,
        now: i64,
    ) -> Result<usize> {
        let conn = self.lock()?;
        let candidates = {
            let mut stmt =
                conn.prepare("SELECT id, pid FROM schedule_run WHERE status = 'running'")?;
            let rows = stmt
                .query_map([], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, Option<i64>>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        let mut changed = 0;
        for (id, pid) in candidates {
            if pid.is_some_and(&alive) {
                continue;
            }
            changed += conn.execute(
                "UPDATE schedule_run SET status = 'interrupted', finished_at = ?1, \
                 error = 'the process stopped before the run finished' WHERE id = ?2",
                rusqlite::params![now, id],
            )?;
        }
        Ok(changed)
    }

    /// Mark one run (or every run, with `None`) read in the results inbox.
    pub fn mark_schedule_runs_read(&self, id: Option<&str>) -> Result<usize> {
        Ok(self.lock()?.execute(
            "UPDATE schedule_run SET read = 1 WHERE read = 0 AND (?1 IS NULL OR id = ?1)",
            [id],
        )?)
    }

    /// Fetch a schedule by exact id.
    pub fn get_schedule(&self, id: &str) -> Result<Option<Schedule>> {
        Ok(self.list_schedules()?.into_iter().find(|s| s.id == id))
    }
}

#[cfg(test)]
mod tests {
    use crate::Store;

    fn store_with_schedule() -> (Store, String) {
        let store = Store::open_in_memory().unwrap();
        let id = forge_types::new_id();
        store
            .add_schedule(&id, "t", "/tmp", None, None, "every:60")
            .unwrap();
        (store, id)
    }

    #[test]
    fn a_run_goes_from_running_to_its_final_status_and_stamps_last_run() {
        let (store, sid) = store_with_schedule();
        store
            .begin_schedule_run("r1", &sid, "timer", 1, Some("sess"), Some(42), 100)
            .unwrap();
        assert_eq!(store.running_schedule_runs(&sid).unwrap().len(), 1);
        store
            .finish_schedule_run("r1", "ok", Some("done"), None, 160)
            .unwrap();
        let runs = store.list_schedule_runs(Some(&sid), 10).unwrap();
        assert_eq!(runs[0].status, "ok");
        assert_eq!(runs[0].finished_at, Some(160));
        assert_eq!(runs[0].summary.as_deref(), Some("done"));
        assert!(!runs[0].read);
        assert_eq!(store.list_schedules().unwrap()[0].last_run, Some(160));
    }

    #[test]
    fn dead_running_rows_become_interrupted_and_live_ones_stay() {
        let (store, sid) = store_with_schedule();
        store
            .begin_schedule_run("dead", &sid, "timer", 1, None, Some(1), 10)
            .unwrap();
        store
            .begin_schedule_run("live", &sid, "timer", 1, None, Some(2), 11)
            .unwrap();
        store
            .begin_schedule_run("nopid", &sid, "timer", 1, None, None, 12)
            .unwrap();
        let changed = store
            .interrupt_dead_schedule_runs(|pid| pid == 2, 50)
            .unwrap();
        assert_eq!(changed, 2);
        let by_id = |id: &str| {
            store
                .list_schedule_runs(None, 10)
                .unwrap()
                .into_iter()
                .find(|r| r.id == id)
                .unwrap()
        };
        assert_eq!(by_id("dead").status, "interrupted");
        assert_eq!(by_id("nopid").status, "interrupted");
        assert_eq!(by_id("live").status, "running");
    }

    #[test]
    fn marking_read_is_per_run_or_global_and_deleting_the_schedule_drops_history() {
        let (store, sid) = store_with_schedule();
        for id in ["a", "b"] {
            store
                .begin_schedule_run(id, &sid, "manual", 1, None, None, 1)
                .unwrap();
            store.finish_schedule_run(id, "ok", None, None, 2).unwrap();
        }
        assert_eq!(store.mark_schedule_runs_read(Some("a")).unwrap(), 1);
        assert_eq!(store.mark_schedule_runs_read(None).unwrap(), 1);
        store.remove_schedule(&sid).unwrap();
        assert!(store.list_schedule_runs(None, 10).unwrap().is_empty());
    }

    #[test]
    fn policy_and_edit_round_trip() {
        let (store, sid) = store_with_schedule();
        let policy = crate::SchedulePolicy {
            tz: Some("Europe/Amsterdam".into()),
            max_runtime_secs: Some(600),
            retries: 2,
            stop_on_error: true,
            stop_when: Some("all tests pass".into()),
            ..Default::default()
        };
        assert!(store.set_schedule_policy(&sid, &policy).unwrap());
        assert!(store
            .update_schedule(
                &sid,
                "new",
                "/var",
                Some("bypass"),
                None,
                "cron:0 3 * * 1-5"
            )
            .unwrap());
        let row = store.get_schedule(&sid).unwrap().unwrap();
        assert_eq!(row.policy, policy);
        assert_eq!(row.task, "new");
        assert_eq!(row.cron, "cron:0 3 * * 1-5");
    }
}
