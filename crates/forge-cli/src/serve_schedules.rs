//! `forge serve`'s schedule registry — the REST surface over `forge schedule`.
//!
//! - `GET  {base}/api/schedules`               every registered row, oldest first
//! - `POST {base}/api/schedules`               create (`{task, cwd?, every?|at?|cron?, mode?, model?, tz?, ...policy}`)
//! - `POST {base}/api/schedules/{id}/update`   rewrite task / cadence / policy and reinstall the timer
//! - `POST {base}/api/schedules/{id}/run`      "Run now": start one tick immediately
//! - `GET  {base}/api/schedule-runs`           run history, newest first (`?schedule_id=&limit=`)
//! - `POST {base}/api/schedule-runs/{id}/read` mark one run read in the results inbox
//! - `POST {base}/api/schedule-runs/read-all`  mark every run read
//! - `POST {base}/api/schedules/{id}/pause`    uninstall the OS timer, keep the row
//! - `POST {base}/api/schedules/{id}/resume`   reinstall the OS timer
//! - `POST {base}/api/schedules/{id}/delete`   uninstall the timer AND drop the row
//!
//! Backed by the real registry — [`forge_store::Store`]'s `schedule` table plus the same per-OS
//! timer installer `forge schedule add` uses ([`crate::cli::commands::schedule`]). There is no
//! parallel daemon-side scheduler: a row created here is byte-identical to one created from the
//! CLI, and `forge schedule list` shows it immediately.
//!
//! Pause is a real pause, not a flag: the systemd/launchd/schtasks timer is uninstalled so the task
//! genuinely stops firing, and `enabled` records that state for `list`.

use std::sync::Arc;

use axum::extract::{Json, Path as AxumPath, Query, State};
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;

use crate::cli::commands::schedule::tick::sweep_dead_runs;
use crate::cli::commands::schedule::{
    install_timer, parse_at, parse_every, uninstall_timer, ScheduleSpec,
};
use crate::serve::{err_response, json_response, DaemonState};
use forge_store::SchedulePolicy;

/// The path an OS timer should exec. The daemon may have been upgraded on disk while it runs, and
/// `current_exe()` then names `<path> (deleted)`, which a timer file would carry forever.
fn persisted_forge_exe() -> String {
    forge_types::self_exe::installed_path()
        .to_string_lossy()
        .into_owned()
}

#[derive(serde::Serialize)]
pub(crate) struct ScheduleRow {
    id: String,
    task: String,
    cwd: String,
    mode: Option<String>,
    model: Option<String>,
    /// The stored spec verbatim (`every:1800` / `daily:09:00` / `cron:<expr>`).
    cron: String,
    /// Human rendering of `cron` — the same text `forge schedule list` prints. For a `cron:` spec
    /// it names the dialect (`cron` vs `OnCalendar`), which is also what tells the client whether
    /// its own cron parser can compute a "next run" for the row.
    spec_label: String,
    enabled: bool,
    created_at: i64,
    last_run: Option<i64>,
    tz: Option<String>,
    max_runtime_secs: Option<i64>,
    max_iterations: Option<i64>,
    retries: i64,
    backoff_secs: i64,
    misfire: String,
    stop_on_error: bool,
    stop_when: Option<String>,
    /// The newest tick, so a row can show what its last run did without a second request.
    latest_run: Option<RunRow>,
}

#[derive(serde::Serialize)]
pub(crate) struct RunRow {
    id: String,
    schedule_id: String,
    started_at: i64,
    finished_at: Option<i64>,
    status: String,
    trigger: String,
    attempt: i64,
    session_id: Option<String>,
    summary: Option<String>,
    error: Option<String>,
    read: bool,
}

fn run_row(run: forge_store::ScheduleRun) -> RunRow {
    RunRow {
        id: run.id,
        schedule_id: run.schedule_id,
        started_at: run.started_at,
        finished_at: run.finished_at,
        status: run.status,
        trigger: run.trigger,
        attempt: run.attempt,
        session_id: run.session_id,
        summary: run.summary,
        error: run.error,
        read: run.read,
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateScheduleRequest {
    task: String,
    /// Where the task runs. Defaults to the daemon's cwd, mirroring `forge schedule add`, which
    /// uses the shell's cwd.
    cwd: Option<String>,
    /// Exactly one of these three must be set: `30m`, `09:00`, or a calendar expression — either
    /// standard 5-field cron (`0 6 * * 1`), which the installer translates to the host's native
    /// trigger, or a systemd `OnCalendar=` string, which is passed through as it always was.
    every: Option<String>,
    at: Option<String>,
    cron: Option<String>,
    mode: Option<String>,
    model: Option<String>,
    /// IANA zone the calendar is read in; unset means the machine's own.
    tz: Option<String>,
    max_runtime_secs: Option<i64>,
    max_iterations: Option<i64>,
    retries: Option<i64>,
    backoff_secs: Option<i64>,
    /// `run` (default) or `skip`.
    misfire: Option<String>,
    stop_on_error: Option<bool>,
    stop_when: Option<String>,
}

impl CreateScheduleRequest {
    fn spec(&self) -> Result<ScheduleSpec, String> {
        match (&self.every, &self.at, &self.cron) {
            (Some(every), None, None) => Ok(ScheduleSpec::Every(
                parse_every(every).map_err(|error| error.to_string())?,
            )),
            (None, Some(at), None) => {
                let (hour, minute) = parse_at(at).map_err(|error| error.to_string())?;
                Ok(ScheduleSpec::Daily { hour, minute })
            }
            (None, None, Some(cron)) => Ok(ScheduleSpec::Cron(cron.clone())),
            (None, None, None) => {
                Err("pass exactly one of every / at / cron, e.g. every: \"30m\"".to_string())
            }
            _ => Err("every, at, and cron are mutually exclusive".to_string()),
        }
    }

    fn policy(&self) -> Result<SchedulePolicy, String> {
        let tz = self
            .tz
            .as_deref()
            .map(str::trim)
            .filter(|tz| !tz.is_empty())
            .map(str::to_string);
        if let Some(tz) = &tz {
            let well_formed = tz.len() <= 64
                && tz
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-' | '/'));
            if !well_formed {
                return Err(format!(
                    "`{tz}` is not a time zone name like Europe/Amsterdam"
                ));
            }
        }
        let positive = |name: &str, value: Option<i64>| match value {
            Some(v) if v <= 0 => Err(format!("{name} must be greater than zero")),
            other => Ok(other),
        };
        let retries = self.retries.unwrap_or(0);
        if !(0..=5).contains(&retries) {
            return Err("retries must be between 0 and 5".to_string());
        }
        let backoff_secs = self.backoff_secs.unwrap_or(60);
        if !(0..=3600).contains(&backoff_secs) {
            return Err("backoff_secs must be between 0 and 3600".to_string());
        }
        let misfire = self.misfire.clone().unwrap_or_else(|| "run".to_string());
        if misfire != "run" && misfire != "skip" {
            return Err("misfire must be `run` or `skip`".to_string());
        }
        let stop_when = self
            .stop_when
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string);
        if stop_when
            .as_deref()
            .is_some_and(|text| text.chars().count() > 500)
        {
            return Err("stop_when is limited to 500 characters".to_string());
        }
        Ok(SchedulePolicy {
            tz,
            max_runtime_secs: positive("max_runtime_secs", self.max_runtime_secs)?,
            max_iterations: positive("max_iterations", self.max_iterations)?,
            retries,
            backoff_secs,
            misfire,
            stop_on_error: self.stop_on_error.unwrap_or(false),
            stop_when,
        })
    }
}

fn schedule_row(schedule: forge_store::Schedule) -> ScheduleRow {
    schedule_row_with(schedule, None)
}

fn schedule_row_with(
    schedule: forge_store::Schedule,
    latest_run: Option<forge_store::ScheduleRun>,
) -> ScheduleRow {
    let spec_label = ScheduleSpec::from_stored(&schedule.cron)
        .map(|spec| spec.describe())
        .unwrap_or_else(|| schedule.cron.clone());
    ScheduleRow {
        id: schedule.id,
        task: schedule.task,
        cwd: schedule.cwd,
        mode: schedule.mode,
        model: schedule.model,
        cron: schedule.cron,
        spec_label,
        enabled: schedule.enabled,
        created_at: schedule.created_at,
        last_run: schedule.last_run,
        tz: schedule.policy.tz,
        max_runtime_secs: schedule.policy.max_runtime_secs,
        max_iterations: schedule.policy.max_iterations,
        retries: schedule.policy.retries,
        backoff_secs: schedule.policy.backoff_secs,
        misfire: schedule.policy.misfire,
        stop_on_error: schedule.policy.stop_on_error,
        stop_when: schedule.policy.stop_when,
        latest_run: latest_run.map(run_row),
    }
}

fn rows_with_latest_run(store: &forge_store::Store) -> Result<Vec<ScheduleRow>, String> {
    let _ = sweep_dead_runs(store);
    let schedules = store.list_schedules().map_err(|error| error.to_string())?;
    let runs = store
        .list_schedule_runs(None, 500)
        .map_err(|error| error.to_string())?;
    Ok(schedules
        .into_iter()
        .map(|schedule| {
            let latest = runs
                .iter()
                .find(|run| run.schedule_id == schedule.id)
                .cloned();
            schedule_row_with(schedule, latest)
        })
        .collect())
}

/// `GET {base}/api/schedules`.
pub(crate) async fn list_schedules(State(state): State<Arc<DaemonState>>) -> Response {
    let store = state.store.clone();
    let result = tokio::task::spawn_blocking(move || rows_with_latest_run(&store)).await;
    match result {
        Ok(Ok(rows)) => json_response(&rows),
        Ok(Err(message)) => err_response(axum::http::StatusCode::INTERNAL_SERVER_ERROR, &message),
        Err(_) => err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "could not read schedules",
        ),
    }
}

/// `POST {base}/api/schedules`. The row is written first and rolled back if the OS timer fails to
/// install — the same ordering `forge schedule add` uses, so a failed create never leaves an
/// orphaned row with no timer behind it.
pub(crate) async fn create_schedule(
    State(state): State<Arc<DaemonState>>,
    Json(request): Json<CreateScheduleRequest>,
) -> Response {
    let store = state.store.clone();
    let default_cwd = state.default_cwd.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<ScheduleRow, String> {
        let task = request.task.trim().to_string();
        if task.is_empty() {
            return Err("a task is required".to_string());
        }
        let spec = request.spec()?;
        let policy = request.policy()?;
        let cwd = request
            .cwd
            .filter(|cwd| !cwd.trim().is_empty())
            .unwrap_or(default_cwd);
        if !std::path::Path::new(&cwd).is_dir() {
            return Err(format!("no such directory: {cwd}"));
        }
        let forge_exe = persisted_forge_exe();

        let id = forge_types::new_id();
        store
            .add_schedule(
                &id,
                &task,
                &cwd,
                request.mode.as_deref(),
                request.model.as_deref(),
                &spec.to_stored(),
            )
            .map_err(|error| error.to_string())?;
        store
            .set_schedule_policy(&id, &policy)
            .map_err(|error| error.to_string())?;
        if let Err(error) = install_timer(&id, &spec, &cwd, &forge_exe, &policy) {
            let _ = store.remove_schedule(&id);
            return Err(format!("installing the OS timer: {error}"));
        }
        store
            .list_schedules()
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(|row| row.id == id)
            .map(schedule_row)
            .ok_or_else(|| "schedule vanished immediately after creation".to_string())
    })
    .await;
    match result {
        Ok(Ok(row)) => json_response(&row),
        Ok(Err(message)) => err_response(axum::http::StatusCode::BAD_REQUEST, &message),
        Err(_) => err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "could not create schedule",
        ),
    }
}

/// What [`mutate_schedule`] should do to a row's OS timer + `enabled` flag.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum ScheduleAction {
    Pause,
    Resume,
    Delete,
}

async fn mutate_schedule(state: Arc<DaemonState>, id: String, action: ScheduleAction) -> Response {
    let store = state.store.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<Option<ScheduleRow>, String> {
        let Some(schedule) = store
            .list_schedules()
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(|row| row.id == id)
        else {
            return Err("no such schedule".to_string());
        };
        match action {
            ScheduleAction::Delete => {
                // Uninstall first: if the timer survives while the row is gone, the OS keeps
                // firing a task nothing knows about.
                uninstall_timer(&schedule.id).map_err(|error| error.to_string())?;
                store
                    .remove_schedule(&schedule.id)
                    .map_err(|error| error.to_string())?;
                Ok(None)
            }
            ScheduleAction::Pause => {
                uninstall_timer(&schedule.id).map_err(|error| error.to_string())?;
                store
                    .set_schedule_enabled(&schedule.id, false)
                    .map_err(|error| error.to_string())?;
                let mut row = schedule;
                row.enabled = false;
                Ok(Some(schedule_row(row)))
            }
            ScheduleAction::Resume => {
                let spec = ScheduleSpec::from_stored(&schedule.cron)
                    .ok_or_else(|| format!("unrecognised schedule spec: {}", schedule.cron))?;
                let forge_exe = persisted_forge_exe();
                install_timer(
                    &schedule.id,
                    &spec,
                    &schedule.cwd,
                    &forge_exe,
                    &schedule.policy,
                )
                .map_err(|error| error.to_string())?;
                store
                    .set_schedule_enabled(&schedule.id, true)
                    .map_err(|error| error.to_string())?;
                let mut row = schedule;
                row.enabled = true;
                Ok(Some(schedule_row(row)))
            }
        }
    })
    .await;
    match result {
        Ok(Ok(Some(row))) => json_response(&row),
        Ok(Ok(None)) => json_response(&serde_json::json!({ "ok": true, "deleted": true })),
        Ok(Err(message)) if message == "no such schedule" => {
            err_response(axum::http::StatusCode::NOT_FOUND, &message)
        }
        Ok(Err(message)) => err_response(axum::http::StatusCode::BAD_REQUEST, &message),
        Err(_) => err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "could not update schedule",
        ),
    }
}

pub(crate) async fn pause_schedule(
    State(state): State<Arc<DaemonState>>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    mutate_schedule(state, id, ScheduleAction::Pause).await
}

pub(crate) async fn resume_schedule(
    State(state): State<Arc<DaemonState>>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    mutate_schedule(state, id, ScheduleAction::Resume).await
}

pub(crate) async fn delete_schedule(
    State(state): State<Arc<DaemonState>>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    mutate_schedule(state, id, ScheduleAction::Delete).await
}

/// Every schedule route: the registry, edit, Run now and the run history.
pub(crate) fn routes(base: &str) -> Router<Arc<DaemonState>> {
    Router::new()
        .route(
            &format!("{base}/api/schedules"),
            get(list_schedules).post(create_schedule),
        )
        .route(
            &format!("{base}/api/schedules/{{id}}/pause"),
            post(pause_schedule),
        )
        .route(
            &format!("{base}/api/schedules/{{id}}/resume"),
            post(resume_schedule),
        )
        .route(
            &format!("{base}/api/schedules/{{id}}/delete"),
            post(delete_schedule),
        )
        .route(
            &format!("{base}/api/schedules/{{id}}/update"),
            post(update_schedule),
        )
        .route(
            &format!("{base}/api/schedules/{{id}}/run"),
            post(run_schedule_now),
        )
        .route(
            &format!("{base}/api/schedule-runs"),
            get(list_schedule_runs),
        )
        .route(
            &format!("{base}/api/schedule-runs/read-all"),
            post(mark_all_runs_read),
        )
        .route(
            &format!("{base}/api/schedule-runs/{{id}}/read"),
            post(mark_run_read),
        )
}

/// `POST {base}/api/schedules/{id}/update`. The timer is reinstalled (when the schedule is
/// running) before the row is rewritten, so a cadence the OS rejects leaves the old schedule
/// untouched.
pub(crate) async fn update_schedule(
    State(state): State<Arc<DaemonState>>,
    AxumPath(id): AxumPath<String>,
    Json(request): Json<CreateScheduleRequest>,
) -> Response {
    let store = state.store.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<ScheduleRow, String> {
        let existing = store
            .get_schedule(&id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "no such schedule".to_string())?;
        let task = request.task.trim().to_string();
        if task.is_empty() {
            return Err("a task is required".to_string());
        }
        let spec = request.spec()?;
        let policy = request.policy()?;
        let cwd = request
            .cwd
            .clone()
            .filter(|cwd| !cwd.trim().is_empty())
            .unwrap_or_else(|| existing.cwd.clone());
        if !std::path::Path::new(&cwd).is_dir() {
            return Err(format!("no such directory: {cwd}"));
        }
        if existing.enabled {
            install_timer(&id, &spec, &cwd, &persisted_forge_exe(), &policy)
                .map_err(|error| format!("installing the OS timer: {error}"))?;
        }
        store
            .update_schedule(
                &id,
                &task,
                &cwd,
                request.mode.as_deref(),
                request.model.as_deref(),
                &spec.to_stored(),
            )
            .map_err(|error| error.to_string())?;
        store
            .set_schedule_policy(&id, &policy)
            .map_err(|error| error.to_string())?;
        rows_with_latest_run(&store)?
            .into_iter()
            .find(|row| row.id == id)
            .ok_or_else(|| "schedule vanished during the update".to_string())
    })
    .await;
    respond(result, "could not update schedule")
}

fn respond<T: serde::Serialize>(
    result: Result<Result<T, String>, tokio::task::JoinError>,
    fallback: &str,
) -> Response {
    match result {
        Ok(Ok(value)) => json_response(&value),
        Ok(Err(message)) if message == "no such schedule" => {
            err_response(axum::http::StatusCode::NOT_FOUND, &message)
        }
        Ok(Err(message)) => err_response(axum::http::StatusCode::BAD_REQUEST, &message),
        Err(_) => err_response(axum::http::StatusCode::INTERNAL_SERVER_ERROR, fallback),
    }
}

/// `POST {base}/api/schedules/{id}/run`: start one tick now, exactly as the OS timer would, but
/// even when the schedule is paused. The tick is a detached `forge schedule tick`, so the run
/// outlives this request and shows up in the history and in Fleet like any other.
pub(crate) async fn run_schedule_now(
    State(state): State<Arc<DaemonState>>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let store = state.store.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<serde_json::Value, String> {
        let schedule = store
            .get_schedule(&id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "no such schedule".to_string())?;
        let mut command = std::process::Command::new(persisted_forge_exe());
        command
            .args(["schedule", "tick", &schedule.id, "--manual"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut command, 0);
        command
            .spawn()
            .map_err(|error| format!("starting the run: {error}"))?;
        Ok(serde_json::json!({ "ok": true, "started": true }))
    })
    .await;
    respond(result, "could not start the run")
}

#[derive(serde::Deserialize)]
pub(crate) struct RunsQuery {
    schedule_id: Option<String>,
    limit: Option<i64>,
}

/// `GET {base}/api/schedule-runs`.
pub(crate) async fn list_schedule_runs(
    State(state): State<Arc<DaemonState>>,
    Query(query): Query<RunsQuery>,
) -> Response {
    let store = state.store.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<Vec<RunRow>, String> {
        let _ = sweep_dead_runs(&store);
        store
            .list_schedule_runs(
                query.schedule_id.as_deref(),
                query.limit.unwrap_or(50).clamp(1, 500),
            )
            .map(|runs| runs.into_iter().map(run_row).collect())
            .map_err(|error| error.to_string())
    })
    .await;
    respond(result, "could not read run history")
}

/// `POST {base}/api/schedule-runs/{id}/read`.
pub(crate) async fn mark_run_read(
    State(state): State<Arc<DaemonState>>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    mark_runs_read(state, Some(id)).await
}

/// `POST {base}/api/schedule-runs/read-all`.
pub(crate) async fn mark_all_runs_read(State(state): State<Arc<DaemonState>>) -> Response {
    mark_runs_read(state, None).await
}

async fn mark_runs_read(state: Arc<DaemonState>, id: Option<String>) -> Response {
    let store = state.store.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<serde_json::Value, String> {
        let changed = store
            .mark_schedule_runs_read(id.as_deref())
            .map_err(|error| error.to_string())?;
        Ok(serde_json::json!({ "ok": true, "changed": changed }))
    })
    .await;
    respond(result, "could not update the inbox")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_render_the_same_spec_label_the_cli_prints() {
        let row = schedule_row(forge_store::Schedule {
            id: "abc".into(),
            task: "review the diff".into(),
            cwd: "/tmp".into(),
            mode: None,
            model: None,
            cron: "every:1800".into(),
            enabled: true,
            created_at: 1,
            last_run: None,
            policy: Default::default(),
        });
        assert_eq!(row.spec_label, "every 30m");
        assert_eq!(row.cron, "every:1800");
    }

    #[test]
    fn cron_rows_label_the_dialect_the_installer_recognised() {
        let row = |cron: &str| {
            schedule_row(forge_store::Schedule {
                id: "abc".into(),
                task: "t".into(),
                cwd: "/tmp".into(),
                mode: None,
                model: None,
                cron: cron.into(),
                enabled: true,
                created_at: 1,
                last_run: None,
                policy: Default::default(),
            })
        };
        assert_eq!(row("cron:0 6 * * 1").spec_label, "cron `0 6 * * 1`");
        // Rows written before the translator existed hold OnCalendar syntax and keep working.
        assert_eq!(
            row("cron:Mon *-*-* 06:00:00").spec_label,
            "OnCalendar `Mon *-*-* 06:00:00`"
        );
    }

    #[test]
    fn unparseable_specs_fall_back_to_the_stored_text() {
        let row = schedule_row(forge_store::Schedule {
            id: "abc".into(),
            task: "t".into(),
            cwd: "/tmp".into(),
            mode: None,
            model: None,
            cron: "nonsense".into(),
            enabled: false,
            created_at: 1,
            last_run: Some(5),
            policy: Default::default(),
        });
        assert_eq!(row.spec_label, "nonsense");
        assert!(!row.enabled);
    }

    fn request(tz: Option<&str>) -> CreateScheduleRequest {
        CreateScheduleRequest {
            task: "t".into(),
            cwd: None,
            every: None,
            at: None,
            cron: Some("0 3 * * 1-5".into()),
            mode: None,
            model: None,
            tz: tz.map(str::to_string),
            max_runtime_secs: None,
            max_iterations: None,
            retries: None,
            backoff_secs: None,
            misfire: None,
            stop_on_error: None,
            stop_when: None,
        }
    }

    #[test]
    fn policy_defaults_and_trims_the_zone() {
        let policy = request(Some(" Asia/Tokyo ")).policy().unwrap();
        assert_eq!(policy.tz.as_deref(), Some("Asia/Tokyo"));
        assert_eq!(policy.misfire, "run");
        assert_eq!(policy.retries, 0);
        assert_eq!(request(Some("  ")).policy().unwrap().tz, None);
    }

    #[test]
    fn policy_rejects_values_the_tick_cannot_honour() {
        assert!(request(Some("Europe/Amsterdam; rm -rf")).policy().is_err());
        let mut bad = request(None);
        bad.retries = Some(6);
        assert!(bad.policy().is_err());
        let mut bad = request(None);
        bad.misfire = Some("later".into());
        assert!(bad.policy().is_err());
        let mut bad = request(None);
        bad.max_runtime_secs = Some(0);
        assert!(bad.policy().is_err());
    }

    #[test]
    fn exactly_one_cadence_is_required() {
        let mut both = request(None);
        both.every = Some("30m".into());
        assert!(both.spec().is_err());
        assert_eq!(
            request(None).spec().unwrap(),
            ScheduleSpec::Cron("0 3 * * 1-5".into())
        );
    }
}
