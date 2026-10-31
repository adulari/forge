//! `forge serve`'s project notes: a markdown scratchpad and a checklist per project.
//!
//! - `GET  {base}/api/notes?project=`             scratchpad plus to-dos (`project` defaults to the daemon's directory)
//! - `PUT  {base}/api/notes`                      save the scratchpad (`{project, body, expected_updated_at?}`)
//! - `POST {base}/api/notes/todos`                add a to-do (`{project, text}`)
//! - `POST {base}/api/notes/todos/{id}/update`    edit the text and/or tick it (`{text?, done?}`)
//! - `POST {base}/api/notes/todos/{id}/link`      remember the session started from it (`{session_id}`)
//! - `POST {base}/api/notes/todos/{id}/delete`    drop it
//! - `POST {base}/api/notes/clear-done`           drop every ticked to-do of a project (`{project}`)
//!
//! Backed by [`forge_store::Store`]'s `project_note` and `project_todo` tables, the same rows the
//! TUI's `/notes` overlay and `forge notes` read, so a to-do added in one place shows up in every
//! other. Every write wakes the open fleet sockets with a `notes_changed` frame so a second
//! device refetches instead of waiting for its poll.

use std::sync::Arc;

use axum::extract::{Json, Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use forge_store::{ProjectNotes, ProjectTodo, StoreError};

use crate::serve::{err_response, json_response, DaemonState};

#[derive(serde::Serialize)]
struct TodoRow {
    id: String,
    text: String,
    done: bool,
    session_id: Option<String>,
    created_at: i64,
    updated_at: i64,
}

#[derive(serde::Serialize)]
struct NotesResponse {
    project: String,
    body: String,
    updated_at: i64,
    todos: Vec<TodoRow>,
}

fn todo_row(todo: ProjectTodo) -> TodoRow {
    TodoRow {
        id: todo.id,
        text: todo.text,
        done: todo.done,
        session_id: todo.session_id,
        created_at: todo.created_at,
        updated_at: todo.updated_at,
    }
}

fn notes_response(notes: ProjectNotes) -> NotesResponse {
    NotesResponse {
        project: notes.project,
        body: notes.body,
        updated_at: notes.updated_at,
        todos: notes.todos.into_iter().map(todo_row).collect(),
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn store_error(error: StoreError) -> Response {
    match error {
        StoreError::InvalidValue(message) => err_response(StatusCode::BAD_REQUEST, &message),
        other => err_response(StatusCode::INTERNAL_SERVER_ERROR, &other.to_string()),
    }
}

/// Run a store call off the async threads and wake the fleet sockets when it changed something.
async fn with_store<T: Send + 'static>(
    state: &Arc<DaemonState>,
    wake: bool,
    run: impl FnOnce(&forge_store::Store) -> Result<T, StoreError> + Send + 'static,
) -> Result<T, Box<Response>> {
    let store = state.store.clone();
    let result = tokio::task::spawn_blocking(move || run(&store))
        .await
        .map_err(|_| {
            Box::new(err_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "notes task failed",
            ))
        })?
        .map_err(|error| Box::new(store_error(error)))?;
    if wake {
        state.registry.signals.notify_notes();
    }
    Ok(result)
}

fn project_or_default(state: &DaemonState, project: Option<String>) -> String {
    project
        .filter(|project| !project.trim().is_empty())
        .unwrap_or_else(|| state.default_cwd.clone())
}

#[derive(serde::Deserialize)]
struct ProjectQuery {
    project: Option<String>,
}

async fn get_notes(
    State(state): State<Arc<DaemonState>>,
    Query(query): Query<ProjectQuery>,
) -> Response {
    let project = project_or_default(&state, query.project);
    match with_store(&state, false, move |store| store.project_notes(&project)).await {
        Ok(notes) => json_response(&notes_response(notes)),
        Err(response) => *response,
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SaveScratchpad {
    project: Option<String>,
    body: String,
    /// The `updated_at` this edit was made on top of. A different stored value means another
    /// device saved in between, and the save is refused rather than overwriting it.
    expected_updated_at: Option<i64>,
}

async fn save_scratchpad(
    State(state): State<Arc<DaemonState>>,
    Json(request): Json<SaveScratchpad>,
) -> Response {
    let project = project_or_default(&state, request.project);
    let SaveScratchpad {
        body,
        expected_updated_at,
        ..
    } = request;
    let result = with_store(&state, true, move |store| {
        let current = store.project_notes(&project)?;
        if let Some(expected) = expected_updated_at {
            if current.updated_at != expected && current.body != body {
                return Ok(Err(current));
            }
        }
        let updated_at = store.set_project_scratchpad(&project, &body, now_ms())?;
        Ok(Ok(updated_at))
    })
    .await;
    match result {
        Ok(Ok(updated_at)) => {
            json_response(&serde_json::json!({ "ok": true, "updated_at": updated_at }))
        }
        Ok(Err(current)) => (
            StatusCode::CONFLICT,
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            serde_json::json!({
                "error": "the scratchpad changed on another device",
                "code": "conflict",
                "body": current.body,
                "updated_at": current.updated_at,
            })
            .to_string(),
        )
            .into_response(),
        Err(response) => *response,
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct AddTodo {
    project: Option<String>,
    text: String,
}

async fn add_todo(State(state): State<Arc<DaemonState>>, Json(request): Json<AddTodo>) -> Response {
    let project = project_or_default(&state, request.project);
    let text = request.text;
    match with_store(&state, true, move |store| {
        store.add_project_todo(&project, &text, now_ms())
    })
    .await
    {
        Ok(todo) => json_response(&todo_row(todo)),
        Err(response) => *response,
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateTodo {
    text: Option<String>,
    done: Option<bool>,
}

async fn update_todo(
    State(state): State<Arc<DaemonState>>,
    AxumPath(id): AxumPath<String>,
    Json(request): Json<UpdateTodo>,
) -> Response {
    match with_store(&state, true, move |store| {
        store.update_project_todo(&id, request.text.as_deref(), request.done, now_ms())
    })
    .await
    {
        Ok(Some(todo)) => json_response(&todo_row(todo)),
        Ok(None) => err_response(StatusCode::NOT_FOUND, "no such to-do"),
        Err(response) => *response,
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct LinkTodo {
    session_id: String,
}

async fn link_todo(
    State(state): State<Arc<DaemonState>>,
    AxumPath(id): AxumPath<String>,
    Json(request): Json<LinkTodo>,
) -> Response {
    match with_store(&state, true, move |store| {
        store.link_project_todo_session(&id, &request.session_id, now_ms())
    })
    .await
    {
        Ok(true) => json_response(&serde_json::json!({ "ok": true })),
        Ok(false) => err_response(StatusCode::NOT_FOUND, "no such to-do"),
        Err(response) => *response,
    }
}

async fn delete_todo(
    State(state): State<Arc<DaemonState>>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    match with_store(&state, true, move |store| store.delete_project_todo(&id)).await {
        Ok(true) => json_response(&serde_json::json!({ "ok": true })),
        Ok(false) => err_response(StatusCode::NOT_FOUND, "no such to-do"),
        Err(response) => *response,
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ClearDone {
    project: Option<String>,
}

async fn clear_done(
    State(state): State<Arc<DaemonState>>,
    Json(request): Json<ClearDone>,
) -> Response {
    let project = project_or_default(&state, request.project);
    match with_store(&state, true, move |store| {
        store.clear_done_project_todos(&project)
    })
    .await
    {
        Ok(removed) => json_response(&serde_json::json!({ "ok": true, "removed": removed })),
        Err(response) => *response,
    }
}

pub(crate) fn routes(base: &str) -> Router<Arc<DaemonState>> {
    Router::new()
        .route(
            &format!("{base}/api/notes"),
            get(get_notes).put(save_scratchpad),
        )
        .route(&format!("{base}/api/notes/todos"), post(add_todo))
        .route(&format!("{base}/api/notes/clear-done"), post(clear_done))
        .route(
            &format!("{base}/api/notes/todos/{{id}}/update"),
            post(update_todo),
        )
        .route(
            &format!("{base}/api/notes/todos/{{id}}/link"),
            post(link_todo),
        )
        .route(
            &format!("{base}/api/notes/todos/{{id}}/delete"),
            post(delete_todo),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    fn state() -> Arc<DaemonState> {
        Arc::new(DaemonState {
            registry: Arc::new(crate::serve::SessionRegistry::new()),
            terminals: Arc::new(crate::serve_terminal::TerminalRegistry::new()),
            store: Arc::new(forge_store::Store::open_in_memory().unwrap()),
            base: "/tok".into(),
            mock: true,
            default_cwd: "/w/app".into(),
            project_roots: Vec::new(),
            push: None,
            apns: None,
            voice: crate::voice::VoiceState::new(),
            anywhere_enable: tokio::sync::watch::channel(false).0,
        })
    }

    async fn call(
        state: &Arc<DaemonState>,
        method: &str,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> (StatusCode, serde_json::Value) {
        let router = routes("/tok").with_state(state.clone());
        let request = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json")
            .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
            .unwrap();
        let response = router.oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
        )
    }

    #[tokio::test]
    async fn todos_round_trip_and_default_to_the_daemon_project() {
        let state = state();
        let (status, todo) = call(
            &state,
            "POST",
            "/tok/api/notes/todos",
            Some(serde_json::json!({"text": "ship it"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let id = todo["id"].as_str().unwrap().to_string();

        let (_, notes) = call(&state, "GET", "/tok/api/notes", None).await;
        assert_eq!(notes["project"], "/w/app");
        assert_eq!(notes["todos"][0]["text"], "ship it");

        let (status, todo) = call(
            &state,
            "POST",
            &format!("/tok/api/notes/todos/{id}/update"),
            Some(serde_json::json!({"done": true})),
        )
        .await;
        assert_eq!(
            (status, todo["done"].clone()),
            (StatusCode::OK, true.into())
        );

        let (status, _) = call(
            &state,
            "POST",
            &format!("/tok/api/notes/todos/{id}/link"),
            Some(serde_json::json!({"session_id": "s1"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (_, notes) = call(&state, "GET", "/tok/api/notes?project=/w/app/", None).await;
        assert_eq!(notes["todos"][0]["session_id"], "s1");

        let (status, _) = call(
            &state,
            "POST",
            &format!("/tok/api/notes/todos/{id}/delete"),
            Some(serde_json::json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = call(
            &state,
            "POST",
            &format!("/tok/api/notes/todos/{id}/delete"),
            Some(serde_json::json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn a_blank_todo_is_a_400() {
        let state = state();
        let (status, body) = call(
            &state,
            "POST",
            "/tok/api/notes/todos",
            Some(serde_json::json!({"text": "  "})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["error"].as_str().unwrap().contains("text"));
    }

    #[tokio::test]
    async fn a_stale_scratchpad_save_is_refused_with_the_current_text() {
        let state = state();
        let (_, saved) = call(
            &state,
            "PUT",
            "/tok/api/notes",
            Some(serde_json::json!({"body": "from the desktop", "expected_updated_at": 0})),
        )
        .await;
        let first = saved["updated_at"].as_i64().unwrap();
        assert!(first > 0);

        let (status, body) = call(
            &state,
            "PUT",
            "/tok/api/notes",
            Some(serde_json::json!({"body": "from the phone", "expected_updated_at": 0})),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body["body"], "from the desktop");
        assert_eq!(body["code"], "conflict");

        let (status, _) = call(
            &state,
            "PUT",
            "/tok/api/notes",
            Some(serde_json::json!({"body": "from the phone", "expected_updated_at": first})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn writes_wake_the_fleet_sockets() {
        let state = state();
        let rx = state.registry.signals.notes_rx();
        call(
            &state,
            "POST",
            "/tok/api/notes/todos",
            Some(serde_json::json!({"text": "x"})),
        )
        .await;
        assert!(rx.has_changed().unwrap());
    }
}
