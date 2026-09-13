//! `POST /api/dispatch`: validate the request and start the coordinator session.

use super::*;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StartDispatchReq {
    prompt: String,
    cwd: Option<String>,
    worktree: Option<bool>,
    mode: Option<String>,
    max_running: Option<i64>,
    max_items: Option<i64>,
    model: Option<String>,
}

#[derive(Debug)]
struct ValidStart {
    prompt: String,
    cwd: String,
    worktree: bool,
    mode: PermissionMode,
    max_running: i64,
    max_items: i64,
    model: Option<String>,
}

fn validate_start(req: StartDispatchReq, default_cwd: &str) -> Result<ValidStart, String> {
    let prompt = req.prompt.trim().to_string();
    if prompt.is_empty() {
        return Err("prompt must not be empty: describe the work to split into sessions".into());
    }
    if prompt.len() > MAX_DISPATCH_PROMPT_BYTES {
        return Err(format!(
            "prompt is {} bytes, over the {MAX_DISPATCH_PROMPT_BYTES}-byte limit: shorten it",
            prompt.len()
        ));
    }
    let max_running = req.max_running.unwrap_or(plan::DEFAULT_MAX_RUNNING as i64);
    if !(1..=plan::MAX_RUNNING_HARD as i64).contains(&max_running) {
        return Err(format!(
            "max_running must be between 1 and {}; got {max_running}",
            plan::MAX_RUNNING_HARD
        ));
    }
    let max_items = req.max_items.unwrap_or(plan::DEFAULT_MAX_ITEMS as i64);
    if !(1..=plan::MAX_ITEMS_HARD as i64).contains(&max_items) {
        return Err(format!(
            "max_items must be between 1 and {}; got {max_items}",
            plan::MAX_ITEMS_HARD
        ));
    }
    let mode = match req.mode.as_deref() {
        None => PermissionMode::AcceptEdits,
        Some(raw) => match PermissionMode::from_key(raw) {
            Some(
                m
                @ (PermissionMode::Default | PermissionMode::AcceptEdits | PermissionMode::Bypass),
            ) => m,
            _ => {
                return Err(format!(
                    "mode: unknown value {raw:?} — valid values: default, accept-edits, bypass"
                ))
            }
        },
    };
    let raw_cwd = req
        .cwd
        .filter(|cwd| !cwd.trim().is_empty())
        .unwrap_or_else(|| default_cwd.to_string());
    let cwd = match std::fs::canonicalize(&raw_cwd) {
        Ok(path) if path.is_dir() => path.display().to_string(),
        _ => return Err(format!("cwd is not a directory: {raw_cwd}")),
    };
    let worktree = req.worktree.unwrap_or(true);
    if worktree && !forge_core::worktree::is_git_repo(std::path::Path::new(&cwd)) {
        return Err(format!(
            "worktree: {cwd} is not a git repository — set \"worktree\": false to run every item \
             in this directory"
        ));
    }
    Ok(ValidStart {
        prompt,
        cwd,
        worktree,
        mode,
        max_running,
        max_items,
        model: req.model.filter(|m| !m.trim().is_empty()),
    })
}

pub(super) fn coordinator_session_title(prompt: &str) -> String {
    let first = prompt.lines().next().unwrap_or("").trim();
    forge_types::truncate_ellipsis(&format!("Dispatch: {first}"), MAX_COORDINATOR_TITLE_CHARS)
}

/// `POST /api/dispatch` — start a coordinator session for one request.
pub(super) async fn start_dispatch(
    State(state): State<Arc<DaemonState>>,
    request: Result<axum::Json<StartDispatchReq>, JsonRejection>,
) -> Response {
    let req = match request {
        Ok(axum::Json(req)) => req,
        Err(error) => {
            return err_response(
                StatusCode::BAD_REQUEST,
                &format!("invalid dispatch request: {}", error.body_text()),
            )
        }
    };
    if state.registry.is_draining() {
        return err_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "daemon is draining for shutdown",
        );
    }
    let v = match validate_start(req, &state.default_cwd) {
        Ok(v) => v,
        Err(message) => return err_response(StatusCode::BAD_REQUEST, &message),
    };
    let title = coordinator_session_title(&v.prompt);
    // The coordinator runs in `default` mode whatever the workers get: it is told not to edit, and
    // any edit it attempts anyway should reach the user as a question.
    let coordinator_req = CreateSessionReq {
        cwd: Some(v.cwd.clone()),
        title: Some(title.clone()),
        model: v.model.clone(),
        mode: Some(PermissionMode::Default.key().to_string()),
        dispatch_coordinator: true,
        dispatch_worker: false,
        ..Default::default()
    };
    let coordinator = match start_session(&state, coordinator_req).await {
        Ok(handle) => handle,
        Err(error) => return error.into_response(),
    };
    let id = forge_types::new_id();
    if let Err(error) = state.store.create_dispatch(
        &id,
        &coordinator.session_id,
        &v.cwd,
        &v.prompt,
        v.worktree,
        Some(v.mode.key()),
        v.max_running,
        v.max_items,
    ) {
        // A coordinator without its dispatch row could never propose; do not leave it in the fleet.
        if let Some(handle) = state.registry.remove(&coordinator.session_id).await {
            handle.shutdown();
        }
        let _ = state
            .store
            .set_session_daemon_live(&coordinator.session_id, false);
        return err_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("recording the dispatch failed: {error}"),
        );
    }
    let first_prompt =
        plan::coordinator_prompt(&v.prompt, &v.cwd, v.max_items as usize, v.worktree);
    let _ = coordinator
        .input_tx
        .send(remote::RemoteInput::Prompt {
            text: first_prompt,
            attachments: Vec::new(),
        })
        .await;
    state.registry.notify_fleet();
    json_response(&serde_json::json!({
        "dispatch_id": id,
        "coordinator_session_id": coordinator.session_id,
        "title": title,
    }))
}
