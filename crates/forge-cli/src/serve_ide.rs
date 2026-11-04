//! The reads behind the desktop IDE mode that no earlier route served:
//!
//! - `GET /api/blame?session=<id>&path=<p>` — which turn wrote each line (`forge blame`, per line)
//! - `GET /api/sessions/{id}/jobs` — the session's background shell jobs
//! - `GET /api/sessions/{id}/jobs/{job}/tail[?lines=<n>]` — the end of one job's log
//!
//! All three are read-only and session-addressed: the directory comes from the session, never from
//! the request, and a requested file must resolve inside the session's repository.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::get;
use axum::Router;

use crate::serve::{err_response, json_response, DaemonState};
use crate::serve_git::{repo_relative, repo_root_or_dir, session_dir};

/// Largest file `GET /api/blame` will read. Attribution is a per-line text match; a generated
/// multi-megabyte file would only produce noise.
const MAX_BLAME_BYTES: u64 = 2 * 1024 * 1024;
/// Prompt characters kept per line's `prompt_summary`.
const PROMPT_SUMMARY_CHARS: usize = 140;
const DEFAULT_TAIL_LINES: usize = 200;
const MAX_TAIL_LINES: usize = 2_000;

pub(crate) fn routes(base: &str) -> Router<Arc<DaemonState>> {
    Router::new()
        .route(&format!("{base}/api/blame"), get(blame))
        .route(&format!("{base}/api/sessions/{{id}}/jobs"), get(jobs))
        .route(
            &format!("{base}/api/sessions/{{id}}/jobs/{{job}}/tail"),
            get(job_tail),
        )
}

#[derive(serde::Deserialize)]
pub(crate) struct BlameQuery {
    #[serde(default)]
    session: String,
    #[serde(default)]
    path: String,
}

/// One attributed line. Lines nobody recorded an edit for (human-written, or older than the
/// store's retention) are not listed; the client treats a missing line number as unknown.
#[derive(Debug, serde::Serialize)]
pub(crate) struct BlameLine {
    line: usize,
    session_id: String,
    /// `seq` of the assistant message whose tool call wrote the line.
    turn: i64,
    model: Option<String>,
    tool_call_id: Option<String>,
    /// The user prompt that started that turn, collapsed to one line.
    prompt_summary: Option<String>,
    created_at: Option<i64>,
}

#[derive(serde::Serialize)]
pub(crate) struct BlameResponse {
    path: String,
    line_count: usize,
    lines: Vec<BlameLine>,
}

fn read_blame_source(root: &Path, rel: &str) -> Result<(PathBuf, String), (StatusCode, String)> {
    let bad = |message: &str| (StatusCode::BAD_REQUEST, message.to_string());
    let full = root.join(rel);
    let canonical = full
        .canonicalize()
        .map_err(|_| (StatusCode::NOT_FOUND, "no such file".to_string()))?;
    let canonical_root = root.canonicalize().map_err(|e| bad(&e.to_string()))?;
    if !canonical.starts_with(&canonical_root) {
        return Err(bad("path must stay inside the repository"));
    }
    let meta = std::fs::metadata(&canonical).map_err(|e| bad(&e.to_string()))?;
    if !meta.is_file() {
        return Err(bad("path is not a file"));
    }
    if meta.len() > MAX_BLAME_BYTES {
        return Err(bad("file is too large to attribute"));
    }
    let text = std::fs::read_to_string(&canonical).map_err(|_| bad("file is not UTF-8 text"))?;
    Ok((canonical, text))
}

fn blame_lines(
    store: &forge_store::Store,
    target: &Path,
    text: &str,
) -> anyhow::Result<Vec<BlameLine>> {
    let attributions = crate::blame::blame_file(store, target, text)?;
    let mut turns: std::collections::HashMap<(String, i64), Option<String>> =
        std::collections::HashMap::new();
    let mut lines = Vec::new();
    for attr in attributions {
        let (Some(session_id), Some(turn)) = (attr.session_id, attr.seq) else {
            continue;
        };
        let prompt_summary = match turns.get(&(session_id.clone(), turn)) {
            Some(summary) => summary.clone(),
            None => {
                let summary = store
                    .turn_context(&session_id, turn)
                    .ok()
                    .and_then(|ctx| crate::blame::prompt_summary(&ctx, PROMPT_SUMMARY_CHARS));
                turns.insert((session_id.clone(), turn), summary.clone());
                summary
            }
        };
        lines.push(BlameLine {
            line: attr.line,
            session_id,
            turn,
            model: attr.model,
            tool_call_id: attr.tool_call_id,
            prompt_summary,
            created_at: attr.created_at,
        });
    }
    Ok(lines)
}

/// `GET /api/blame?session=<id>&path=<repo-relative path>`.
async fn blame(
    State(state): State<Arc<DaemonState>>,
    Query(params): Query<BlameQuery>,
) -> Response {
    let Some(dir) = session_dir(&state, &params.session).await else {
        return err_response(StatusCode::NOT_FOUND, "no such session");
    };
    let rel = match repo_relative(&params.path) {
        Ok(rel) => rel,
        Err(message) => return err_response(StatusCode::BAD_REQUEST, &message),
    };
    let store = state.store.clone();
    let result = tokio::task::spawn_blocking(move || {
        let root = repo_root_or_dir(&dir);
        let (target, text) = read_blame_source(&root, &rel)?;
        let lines = blame_lines(&store, &target, &text)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        Ok::<_, (StatusCode, String)>(BlameResponse {
            path: rel,
            line_count: text.lines().count(),
            lines,
        })
    })
    .await;
    match result {
        Ok(Ok(response)) => json_response(&response),
        Ok(Err((status, message))) => err_response(status, &message),
        Err(_) => err_response(StatusCode::INTERNAL_SERVER_ERROR, "blame failed"),
    }
}

#[derive(serde::Serialize)]
struct JobRow {
    id: u32,
    command: String,
    running: bool,
    /// `running`, `exited N` or `gone`.
    status: String,
    started_at: u64,
}

#[derive(serde::Serialize)]
struct JobsResponse {
    jobs: Vec<JobRow>,
}

/// `GET /api/sessions/{id}/jobs` — newest first, running and finished alike.
async fn jobs(State(state): State<Arc<DaemonState>>, AxumPath(id): AxumPath<String>) -> Response {
    let Some(dir) = session_dir(&state, &id).await else {
        return err_response(StatusCode::NOT_FOUND, "no such session");
    };
    let rows = tokio::task::spawn_blocking(move || {
        forge_tools::job_views(Path::new(&dir))
            .into_iter()
            .map(|job| JobRow {
                id: job.id,
                command: job.command,
                running: job.running,
                status: job.status,
                started_at: job.started_at,
            })
            .collect::<Vec<_>>()
    })
    .await;
    match rows {
        Ok(jobs) => json_response(&JobsResponse { jobs }),
        Err(_) => err_response(StatusCode::INTERNAL_SERVER_ERROR, "listing jobs failed"),
    }
}

#[derive(serde::Deserialize)]
struct TailQuery {
    #[serde(default)]
    lines: Option<usize>,
}

#[derive(serde::Serialize)]
struct TailResponse {
    id: u32,
    running: bool,
    status: String,
    text: String,
}

/// `GET /api/sessions/{id}/jobs/{job}/tail?lines=<n>` — the last `n` lines of the job's log
/// (default 200, at most 2000).
async fn job_tail(
    State(state): State<Arc<DaemonState>>,
    AxumPath((id, job)): AxumPath<(String, u32)>,
    Query(query): Query<TailQuery>,
) -> Response {
    let Some(dir) = session_dir(&state, &id).await else {
        return err_response(StatusCode::NOT_FOUND, "no such session");
    };
    let lines = query
        .lines
        .unwrap_or(DEFAULT_TAIL_LINES)
        .clamp(1, MAX_TAIL_LINES);
    let result = tokio::task::spawn_blocking(move || {
        let view = forge_tools::job_views(Path::new(&dir))
            .into_iter()
            .find(|view| view.id == job)?;
        let text = forge_tools::job_tail(&view.log, lines).unwrap_or_else(|e| e);
        Some(TailResponse {
            id: view.id,
            running: view.running,
            status: view.status,
            text,
        })
    })
    .await;
    match result {
        Ok(Some(response)) => json_response(&response),
        Ok(None) => err_response(StatusCode::NOT_FOUND, "no such job"),
        Err(_) => err_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "reading the job log failed",
        ),
    }
}

#[cfg(test)]
mod tests;
