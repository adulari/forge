//! The daemon half of `forge worktree reclaim`: the same classifier (`forge_core::worktree_reclaim`)
//! behind the "Reclaim" offer that a host-pressure warning or refusal carries. Nothing here
//! decides what is safe to delete; it only calls the CLI's survey, removal and prune with the
//! daemon's own idea of which sessions are alive.

use std::path::{Path, PathBuf};
use std::time::Duration;

use forge_core::worktree_reclaim::{self, ReclaimReport};

use crate::cli::commands::worktree::DEFAULT_ARTIFACT_MIN_AGE_HOURS;
use crate::serve::DaemonState;

/// A survey walks every worktree's `target/`; past this the offer is made without a number.
const PREVIEW_BUDGET: Duration = Duration::from_secs(4);

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub(crate) struct Outcome {
    pub freed_bytes: u64,
    pub removed: usize,
    pub pruned: usize,
    pub failures: Vec<String>,
}

async fn live_paths(state: &DaemonState) -> Vec<PathBuf> {
    let store = state.store.clone();
    let mut live: Vec<PathBuf> = tokio::task::spawn_blocking(move || store.live_session_paths())
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default()
        .into_iter()
        .map(PathBuf::from)
        .collect();
    for handle in state.registry.all().await {
        live.push(PathBuf::from(handle.cwd()));
        live.extend(handle.worktree.as_deref().map(PathBuf::from));
    }
    live
}

fn survey(repo: &Path, live: &[PathBuf]) -> Option<ReclaimReport> {
    let min_age = DEFAULT_ARTIFACT_MIN_AGE_HOURS.saturating_mul(3600);
    worktree_reclaim::survey_with_artifact_age(repo, live, min_age).ok()
}

/// Bytes `forge worktree reclaim --artifacts` would free for `repo`; `None` when the survey fails
/// or runs out of [`PREVIEW_BUDGET`].
pub(crate) async fn preview(state: &DaemonState, repo: &Path) -> Option<u64> {
    let live = live_paths(state).await;
    let repo = repo.to_path_buf();
    let task = tokio::task::spawn_blocking(move || survey(&repo, &live));
    let report = tokio::time::timeout(PREVIEW_BUDGET, task)
        .await
        .ok()?
        .ok()??;
    Some(report.reclaimable_bytes())
}

/// Remove every worktree the classifier clears and prune the stale `target/` dirs, exactly as
/// `forge worktree reclaim --artifacts --yes` does.
pub(crate) async fn apply(state: &DaemonState, repo: &Path) -> Result<Outcome, String> {
    let live = live_paths(state).await;
    let repo = repo.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let report = survey(&repo, &live).ok_or("could not survey the repository's worktrees")?;
        Ok(reclaim_report(&repo, &report, &live))
    })
    .await
    .map_err(|e| format!("reclaim task failed: {e}"))?
}

fn reclaim_report(repo: &Path, report: &ReclaimReport, live: &[PathBuf]) -> Outcome {
    let min_age = DEFAULT_ARTIFACT_MIN_AGE_HOURS.saturating_mul(3600);
    let mut out = Outcome::default();
    for candidate in report.reclaimable() {
        match worktree_reclaim::remove(repo, &candidate.facts) {
            Ok(()) => {
                out.removed += 1;
                out.freed_bytes = out.freed_bytes.saturating_add(candidate.facts.size_bytes);
            }
            Err(e) => out
                .failures
                .push(format!("{}: {e}", candidate.facts.path.display())),
        }
    }
    for candidate in report
        .candidates
        .iter()
        .filter(|c| c.artifact_verdict.is_reclaim())
    {
        if candidate.verdict.is_reclaim() && !candidate.facts.path.exists() {
            continue;
        }
        match worktree_reclaim::prune_artifacts(&candidate.facts.path, live, min_age) {
            Ok(bytes) => {
                out.pruned += 1;
                out.freed_bytes = out.freed_bytes.saturating_add(bytes);
            }
            Err(e) => out
                .failures
                .push(format!("{}/target: {e}", candidate.facts.path.display())),
        }
    }
    out
}

/// The repository whose worktrees a request is about: `cwd` itself, or the main checkout when it
/// is one of its linked worktrees.
pub(crate) fn repo_for(cwd: &Path) -> Option<PathBuf> {
    crate::serve_worktree_start::repo_root_of(cwd)
}

/// [`preview`] for the repository containing `cwd`.
pub(crate) async fn preview_for(state: &DaemonState, cwd: &Path) -> Option<u64> {
    let cwd = cwd.to_path_buf();
    let repo = tokio::task::spawn_blocking(move || repo_for(&cwd))
        .await
        .ok()??;
    preview(state, &repo).await
}

/// A directory the daemon already works in: the default project, a project root, or a live
/// session's workspace. Reclaim deletes files, so it does not take an arbitrary path.
async fn known_workspace(state: &DaemonState, cwd: &Path) -> bool {
    let Ok(cwd) = cwd.canonicalize() else {
        return false;
    };
    let mut known = vec![PathBuf::from(&state.default_cwd)];
    known.extend(state.project_roots.iter().cloned());
    for handle in state.registry.all().await {
        known.push(PathBuf::from(handle.cwd()));
        known.extend(handle.worktree.as_deref().map(PathBuf::from));
    }
    known
        .iter()
        .filter_map(|k| k.canonicalize().ok())
        .any(|k| cwd.starts_with(&k) || k.starts_with(&cwd))
}

pub(crate) fn routes(base: &str) -> axum::Router<std::sync::Arc<DaemonState>> {
    use axum::routing::{get, post};
    axum::Router::new()
        .route(&format!("{base}/api/host/pressure"), get(pressure_route))
        .route(&format!("{base}/api/host/reclaim"), post(reclaim_route))
}

#[derive(serde::Deserialize)]
struct CwdQuery {
    cwd: Option<String>,
}

async fn resolve(
    state: &DaemonState,
    cwd: Option<String>,
) -> Result<PathBuf, (axum::http::StatusCode, String)> {
    use axum::http::StatusCode;
    let cwd = PathBuf::from(
        cwd.filter(|c| !c.trim().is_empty())
            .unwrap_or_else(|| state.default_cwd.clone()),
    );
    if !known_workspace(state, &cwd).await {
        return Err((
            StatusCode::FORBIDDEN,
            "cwd is not a workspace this daemon works in".to_string(),
        ));
    }
    let probe = cwd.clone();
    match tokio::task::spawn_blocking(move || repo_for(&probe))
        .await
        .ok()
        .flatten()
    {
        Some(repo) => Ok(repo),
        None => Err((
            StatusCode::BAD_REQUEST,
            format!("{} is not inside a git repository", cwd.display()),
        )),
    }
}

/// `GET /api/host/pressure?cwd=` — the latest sample plus what Reclaim would free for the
/// repository at `cwd`.
async fn pressure_route(
    axum::extract::State(state): axum::extract::State<std::sync::Arc<DaemonState>>,
    axum::extract::Query(query): axum::extract::Query<CwdQuery>,
) -> axum::response::Response {
    let repo = match resolve(&state, query.cwd).await {
        Ok(repo) => repo,
        Err((status, message)) => return crate::serve::err_response(status, &message),
    };
    let reclaimable = preview(&state, &repo).await;
    crate::serve::json_response(&serde_json::json!({
        "pressure": crate::host_pressure::current(),
        "repo": repo,
        "reclaimable_bytes": reclaimable,
    }))
}

#[derive(serde::Deserialize)]
struct ReclaimBody {
    cwd: Option<String>,
}

/// `POST /api/host/reclaim {cwd}` — remove what `forge worktree reclaim --artifacts --yes` would.
async fn reclaim_route(
    axum::extract::State(state): axum::extract::State<std::sync::Arc<DaemonState>>,
    axum::Json(body): axum::Json<ReclaimBody>,
) -> axum::response::Response {
    let repo = match resolve(&state, body.cwd).await {
        Ok(repo) => repo,
        Err((status, message)) => return crate::serve::err_response(status, &message),
    };
    match apply(&state, &repo).await {
        Ok(outcome) => {
            crate::host_pressure::request_resample();
            crate::serve::json_response(&outcome)
        }
        Err(error) => {
            crate::serve::err_response(axum::http::StatusCode::INTERNAL_SERVER_ERROR, &error)
        }
    }
}
