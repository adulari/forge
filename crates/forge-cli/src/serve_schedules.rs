//! `forge serve`'s schedule registry — the REST surface over `forge schedule`.
//!
//! - `GET  {base}/api/schedules`               every registered row, oldest first
//! - `POST {base}/api/schedules`               create (`{task, cwd?, every?|at?|cron?, mode?, model?}`)
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

use axum::extract::{Json, Path as AxumPath, State};
use axum::response::Response;

use crate::cli::commands::schedule::{
    install_timer, parse_at, parse_every, uninstall_timer, ScheduleSpec,
};
use crate::serve::{err_response, json_response, DaemonState};

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
}

fn schedule_row(schedule: forge_store::Schedule) -> ScheduleRow {
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
    }
}

/// `GET {base}/api/schedules`.
pub(crate) async fn list_schedules(State(state): State<Arc<DaemonState>>) -> Response {
    let store = state.store.clone();
    let result = tokio::task::spawn_blocking(move || {
        store
            .list_schedules()
            .map(|rows| rows.into_iter().map(schedule_row).collect::<Vec<_>>())
            .map_err(|error| error.to_string())
    })
    .await;
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
        let spec = match (&request.every, &request.at, &request.cron) {
            (Some(every), None, None) => {
                ScheduleSpec::Every(parse_every(every).map_err(|error| error.to_string())?)
            }
            (None, Some(at), None) => {
                let (hour, minute) = parse_at(at).map_err(|error| error.to_string())?;
                ScheduleSpec::Daily { hour, minute }
            }
            (None, None, Some(cron)) => ScheduleSpec::Cron(cron.clone()),
            (None, None, None) => {
                return Err("pass exactly one of every / at / cron, e.g. every: \"30m\"".to_string())
            }
            _ => return Err("every, at, and cron are mutually exclusive".to_string()),
        };
        let cwd = request
            .cwd
            .filter(|cwd| !cwd.trim().is_empty())
            .unwrap_or(default_cwd);
        if !std::path::Path::new(&cwd).is_dir() {
            return Err(format!("no such directory: {cwd}"));
        }
        let forge_exe = std::env::current_exe()
            .map_err(|error| format!("resolving the forge binary path: {error}"))?
            .to_string_lossy()
            .to_string();

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
        if let Err(error) = install_timer(
            &id,
            &spec,
            &task,
            &cwd,
            request.mode.as_deref(),
            request.model.as_deref(),
            &forge_exe,
        ) {
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
                let forge_exe = std::env::current_exe()
                    .map_err(|error| format!("resolving the forge binary path: {error}"))?
                    .to_string_lossy()
                    .to_string();
                install_timer(
                    &schedule.id,
                    &spec,
                    &schedule.task,
                    &schedule.cwd,
                    schedule.mode.as_deref(),
                    schedule.model.as_deref(),
                    &forge_exe,
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
        });
        assert_eq!(row.spec_label, "nonsense");
        assert!(!row.enabled);
    }
}
