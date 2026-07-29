//! Schedule, queue, and saved-workflow execution persistence.

use super::*;

impl Store {
    // --- forge schedule: recurring OS-timer-driven `forge run` registry ---

    /// Register a new schedule row. `id` is the caller-generated [`forge_types::new_id`] so the CLI
    /// can print/use it before (and regardless of) the store round-trip.
    #[allow(clippy::too_many_arguments)]
    pub fn add_schedule(
        &self,
        id: &str,
        task: &str,
        cwd: &str,
        mode: Option<&str>,
        model: Option<&str>,
        cron: &str,
    ) -> Result<()> {
        self.lock()?.execute(
            "INSERT INTO schedule (id, task, cwd, mode, model, cron) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            (id, task, cwd, mode, model, cron),
        )?;
        Ok(())
    }

    /// All registered schedules, oldest first.
    pub fn list_schedules(&self) -> Result<Vec<Schedule>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id, task, cwd, mode, model, cron, enabled, created_at, last_run \
             FROM schedule ORDER BY created_at",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(Schedule {
                    id: r.get(0)?,
                    task: r.get(1)?,
                    cwd: r.get(2)?,
                    mode: r.get(3)?,
                    model: r.get(4)?,
                    cron: r.get(5)?,
                    enabled: r.get::<_, i64>(6)? != 0,
                    created_at: r.get(7)?,
                    last_run: r.get(8)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Schedule ids whose id starts with `prefix` (git-style prefix resolution, mirrors
    /// [`Store::matching_session_ids`]).
    pub fn matching_schedule_ids(&self, prefix: &str) -> Result<Vec<String>> {
        let conn = self.lock()?;
        let escaped = escape_like_pattern(prefix);
        let mut stmt =
            conn.prepare("SELECT id FROM schedule WHERE id LIKE ?1 || '%' ESCAPE '\\'")?;
        let rows = stmt.query_map([escaped], |row| row.get::<_, String>(0))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// Delete a schedule row by its exact id. Returns `false` if no row matched.
    pub fn remove_schedule(&self, id: &str) -> Result<bool> {
        let n = self
            .lock()?
            .execute("DELETE FROM schedule WHERE id = ?1", [id])?;
        Ok(n > 0)
    }

    /// Flip a schedule's `enabled` flag. Pausing does NOT stop the OS timer by itself — the caller
    /// must uninstall/reinstall it (see `forge serve`'s schedules API); this only records the
    /// state `forge schedule list` reports. Returns `false` if no row matched.
    pub fn set_schedule_enabled(&self, id: &str, enabled: bool) -> Result<bool> {
        let n = self.lock()?.execute(
            "UPDATE schedule SET enabled = ?1 WHERE id = ?2",
            (i64::from(enabled), id),
        )?;
        Ok(n > 0)
    }

    /// Record the epoch-seconds timestamp of a schedule's most recent tick.
    pub fn set_schedule_last_run(&self, id: &str, at: i64) -> Result<()> {
        self.lock()?
            .execute("UPDATE schedule SET last_run = ?1 WHERE id = ?2", (at, id))?;
        Ok(())
    }

    // --- forge queue: the overnight-autopilot task queue ---

    /// Enqueue a task. `id` is caller-generated ([`forge_types::new_id`]) so the CLI can print it
    /// immediately; the row starts in `pending`.
    pub fn add_queue_task(
        &self,
        id: &str,
        task: &str,
        cwd: &str,
        mode: Option<&str>,
        model: Option<&str>,
        budget_usd: Option<f64>,
    ) -> Result<()> {
        self.lock()?.execute(
            "INSERT INTO queue_task (id, task, cwd, mode, model, budget_usd) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            (id, task, cwd, mode, model, budget_usd),
        )?;
        Ok(())
    }

    /// All queue tasks, oldest first. `cwd` filters to one project when given (a drain only runs
    /// the current repo's tasks; `forge queue list` shows everything with `None`).
    pub fn list_queue_tasks(&self, cwd: Option<&str>) -> Result<Vec<QueueTask>> {
        let conn = self.lock()?;
        let sql = "SELECT id, task, cwd, mode, model, budget_usd, status, created_at, \
                   started_at, finished_at, session_id, branch, summary, cost_usd, gate \
                   FROM queue_task";
        let map = |r: &rusqlite::Row<'_>| {
            Ok(QueueTask {
                id: r.get(0)?,
                task: r.get(1)?,
                cwd: r.get(2)?,
                mode: r.get(3)?,
                model: r.get(4)?,
                budget_usd: r.get(5)?,
                status: r.get(6)?,
                created_at: r.get(7)?,
                started_at: r.get(8)?,
                finished_at: r.get(9)?,
                session_id: r.get(10)?,
                branch: r.get(11)?,
                summary: r.get(12)?,
                cost_usd: r.get(13)?,
                gate: r.get(14)?,
            })
        };
        let rows = match cwd {
            Some(dir) => {
                let mut stmt =
                    conn.prepare(&format!("{sql} WHERE cwd = ?1 ORDER BY created_at"))?;
                let rows = stmt.query_map([dir], map)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()?
            }
            None => {
                let mut stmt = conn.prepare(&format!("{sql} ORDER BY created_at"))?;
                let rows = stmt.query_map([], map)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()?
            }
        };
        Ok(rows)
    }

    /// Queue-task ids starting with `prefix` (git-style prefix resolution).
    pub fn matching_queue_task_ids(&self, prefix: &str) -> Result<Vec<String>> {
        let conn = self.lock()?;
        let escaped = escape_like_pattern(prefix);
        let mut stmt =
            conn.prepare("SELECT id FROM queue_task WHERE id LIKE ?1 || '%' ESCAPE '\\'")?;
        let rows = stmt.query_map([escaped], |row| row.get::<_, String>(0))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// Delete a queue task by exact id, but never one mid-run. Returns `false` if nothing matched
    /// (wrong id, or the row is `running`).
    pub fn remove_queue_task(&self, id: &str) -> Result<bool> {
        let n = self.lock()?.execute(
            "DELETE FROM queue_task WHERE id = ?1 AND status != 'running'",
            [id],
        )?;
        Ok(n > 0)
    }

    /// Move a pending task to `running`, stamping `started_at`. Returns `false` when the row was
    /// not pending (already claimed by a concurrent drain, or finished) — the caller skips it.
    pub fn claim_queue_task(&self, id: &str, at: i64) -> Result<bool> {
        let n = self.lock()?.execute(
            "UPDATE queue_task SET status = 'running', started_at = ?1 \
             WHERE id = ?2 AND status = 'pending'",
            (at, id),
        )?;
        Ok(n > 0)
    }

    /// Record a finished task's outcome in one write. Only the worker that successfully claimed
    /// the task may transition it out of `running`; late or duplicate finishers are ignored.
    #[allow(clippy::too_many_arguments)]
    pub fn finish_queue_task(
        &self,
        id: &str,
        status: &str,
        at: i64,
        session_id: Option<&str>,
        branch: Option<&str>,
        summary: Option<&str>,
        cost_usd: Option<f64>,
        gate: Option<&str>,
    ) -> Result<()> {
        self.lock()?.execute(
            "UPDATE queue_task SET status = ?1, finished_at = ?2, session_id = ?3, \
             branch = ?4, summary = ?5, cost_usd = ?6, gate = ?7 \
             WHERE id = ?8 AND status = 'running'",
            (status, at, session_id, branch, summary, cost_usd, gate, id),
        )?;
        Ok(())
    }

    // --- saved-workflow run history (`/workflow run <name>`) ---

    /// Open a `workflow_run` row for a starting saved workflow. `id` is caller-generated
    /// ([`forge_types::new_id`]) so the caller can close the row out later without a round-trip.
    /// `cwd` is the workspace root the script runs against — the same key the workflow library
    /// screen lists scripts by, so one project's history never shows up under another's.
    ///
    /// Also sweeps all long-dead `running` rows (see [`WORKFLOW_RUN_STALE_SECS`]): a row left open
    /// by a killed process is repaired the next time any workflow runs, so the projection [`list_workflow_runs`](Self::list_workflow_runs) applies on read doesn't have to
    /// be re-derived forever.
    pub fn start_workflow_run(
        &self,
        id: &str,
        name: &str,
        session_id: &str,
        cwd: &str,
    ) -> Result<()> {
        let stale_before = chrono::Utc::now().timestamp() - WORKFLOW_RUN_STALE_SECS;
        let conn = self.lock()?;
        conn.execute(
            "UPDATE workflow_run SET status = 'interrupted' \
             WHERE status = 'running' AND started_at < ?1",
            [stale_before],
        )?;
        conn.execute(
            "INSERT INTO workflow_run (id, name, session_id, cwd) VALUES (?1, ?2, ?3, ?4)",
            (id, name, session_id, cwd),
        )?;
        Ok(())
    }

    /// Close a run out with what it ended up doing. `ok` distinguishes `ok` from `failed`; the
    /// counts and cost are the run's own observed totals, not estimates (see
    /// `Session::run_saved_workflow`). No-op if the row is gone (its session was pruned).
    pub fn finish_workflow_run(
        &self,
        id: &str,
        ok: bool,
        summary: &str,
        phases: i64,
        agents: i64,
        cost_usd: f64,
    ) -> Result<()> {
        self.lock()?.execute(
            "UPDATE workflow_run \
             SET status = ?1, finished_at = ?2, summary = ?3, phases = ?4, agents = ?5, \
                 cost_usd = ?6 \
             WHERE id = ?7 AND status = 'running'",
            (
                if ok { "ok" } else { "failed" },
                chrono::Utc::now().timestamp(),
                summary,
                phases,
                agents,
                cost_usd,
                id,
            ),
        )?;
        Ok(())
    }

    /// Mark a run interrupted — the turn was aborted (Esc) or the process is shutting down, so no
    /// outcome exists. Unlike a crash this DOES know when the run stopped, so `finished_at` is
    /// recorded; a crash-interrupted row keeps a NULL `finished_at` because that moment was never
    /// observed. Guarded on `status = 'running'` so it can never overwrite a real outcome.
    pub fn interrupt_workflow_run(&self, id: &str) -> Result<()> {
        self.lock()?.execute(
            "UPDATE workflow_run SET status = 'interrupted', finished_at = ?1 \
             WHERE id = ?2 AND status = 'running'",
            (chrono::Utc::now().timestamp(), id),
        )?;
        Ok(())
    }

    /// The newest `limit` recorded runs of one workflow in one workspace, newest first.
    ///
    /// A `running` row older than [`WORKFLOW_RUN_STALE_SECS`] is REPORTED as `interrupted` (its
    /// `finished_at` stays NULL — the end time was never observed and is not invented). That is
    /// the read-side half of the staleness rule; [`start_workflow_run`](Self::start_workflow_run)
    /// writes the same verdict back to disk on the next run.
    pub fn list_workflow_runs(
        &self,
        name: &str,
        cwd: &str,
        limit: usize,
    ) -> Result<Vec<WorkflowRun>> {
        let stale_before = chrono::Utc::now().timestamp() - WORKFLOW_RUN_STALE_SECS;
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id, name, session_id, cwd, started_at, finished_at, status, summary, \
                    phases, agents, cost_usd \
             FROM workflow_run WHERE name = ?1 AND cwd = ?2 \
             ORDER BY started_at DESC, rowid DESC LIMIT ?3",
        )?;
        let rows = stmt.query_map(rusqlite::params![name, cwd, limit as i64], |r| {
            let status: String = r.get(6)?;
            let started_at: i64 = r.get(4)?;
            Ok(WorkflowRun {
                id: r.get(0)?,
                name: r.get(1)?,
                session_id: r.get(2)?,
                cwd: r.get(3)?,
                started_at,
                finished_at: r.get(5)?,
                status: if status == "running" && started_at < stale_before {
                    "interrupted".to_string()
                } else {
                    status
                },
                summary: r.get(7)?,
                phases: r.get(8)?,
                agents: r.get(9)?,
                cost_usd: r.get(10)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }
}
