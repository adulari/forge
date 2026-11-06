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

/// Every repository this daemon works in, for a request that names no `cwd`: the default project,
/// each project root (itself, or the repos directly under it), and the workspace of every live
/// session. The daemon's own cwd is often `$HOME` or `/`, so it cannot stand for them all.
async fn all_known_repos(state: &DaemonState) -> Vec<PathBuf> {
    let mut candidates = vec![PathBuf::from(&state.default_cwd)];
    candidates.extend(state.project_roots.iter().cloned());
    for handle in state.registry.all().await {
        candidates.push(PathBuf::from(handle.cwd()));
        candidates.extend(handle.worktree.as_deref().map(PathBuf::from));
    }
    let store = state.store.clone();
    let live = tokio::task::spawn_blocking(move || store.live_session_paths())
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default();
    candidates.extend(live.into_iter().map(PathBuf::from));
    let is_root: Vec<PathBuf> = state.project_roots.clone();
    tokio::task::spawn_blocking(move || {
        let mut repos: Vec<PathBuf> = Vec::new();
        let mut add = |repo: PathBuf| {
            let repo = repo.canonicalize().unwrap_or(repo);
            if !repos.contains(&repo) {
                repos.push(repo);
            }
        };
        for candidate in candidates {
            if let Some(repo) = repo_for(&candidate) {
                add(repo);
            } else if is_root.contains(&candidate) {
                let Ok(entries) = std::fs::read_dir(&candidate) else {
                    continue;
                };
                for entry in entries.flatten() {
                    let child = entry.path();
                    if child.join(".git").exists() {
                        if let Some(repo) = repo_for(&child) {
                            add(repo);
                        }
                    }
                }
            }
        }
        repos
    })
    .await
    .unwrap_or_default()
}

/// The repositories a request is about: the one containing `cwd`, or with no `cwd` every
/// repository the daemon knows.
async fn resolve(
    state: &DaemonState,
    cwd: Option<String>,
) -> Result<Vec<PathBuf>, (axum::http::StatusCode, String)> {
    use axum::http::StatusCode;
    let Some(cwd) = cwd.filter(|c| !c.trim().is_empty()).map(PathBuf::from) else {
        let repos = all_known_repos(state).await;
        return if repos.is_empty() {
            Err((
                StatusCode::BAD_REQUEST,
                "no git repository is known to this daemon".to_string(),
            ))
        } else {
            Ok(repos)
        };
    };
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
        Some(repo) => Ok(vec![repo]),
        None => Err((
            StatusCode::BAD_REQUEST,
            format!("{} is not inside a git repository", cwd.display()),
        )),
    }
}

/// [`preview`] summed over `repos`; `None` only when no repository could be surveyed.
async fn preview_all(state: &DaemonState, repos: &[PathBuf]) -> Option<u64> {
    let mut total = None;
    for repo in repos {
        if let Some(bytes) = preview(state, repo).await {
            total = Some(total.unwrap_or(0u64).saturating_add(bytes));
        }
    }
    total
}

/// [`apply`] over `repos`, outcomes summed; a repository that cannot be surveyed is a failure
/// line, not an abort. Errors only when none could be.
async fn apply_all(state: &DaemonState, repos: &[PathBuf]) -> Result<Outcome, String> {
    let mut sum = Outcome::default();
    let mut last_error = None;
    let mut done = 0usize;
    for repo in repos {
        match apply(state, repo).await {
            Ok(out) => {
                done += 1;
                sum.freed_bytes = sum.freed_bytes.saturating_add(out.freed_bytes);
                sum.removed += out.removed;
                sum.pruned += out.pruned;
                sum.failures.extend(out.failures);
            }
            Err(e) => {
                sum.failures.push(format!("{}: {e}", repo.display()));
                last_error = Some(e);
            }
        }
    }
    match (done, last_error) {
        (0, Some(e)) => Err(e),
        _ => Ok(sum),
    }
}

/// `GET /api/host/pressure?cwd=` — the latest sample plus what Reclaim would free for the
/// repository at `cwd`, or for every known repository without one.
async fn pressure_route(
    axum::extract::State(state): axum::extract::State<std::sync::Arc<DaemonState>>,
    axum::extract::Query(query): axum::extract::Query<CwdQuery>,
) -> axum::response::Response {
    let repos = match resolve(&state, query.cwd).await {
        Ok(repos) => repos,
        Err((status, message)) => return crate::serve::err_response(status, &message),
    };
    let reclaimable = preview_all(&state, &repos).await;
    crate::serve::json_response(&serde_json::json!({
        "pressure": crate::host_pressure::current(),
        "repo": repos.first(),
        "repos": repos,
        "reclaimable_bytes": reclaimable,
    }))
}

#[derive(serde::Deserialize)]
struct ReclaimBody {
    cwd: Option<String>,
}

/// `POST /api/host/reclaim {cwd}` — remove what `forge worktree reclaim --artifacts --yes` would,
/// in the repository at `cwd` or in every known repository without one.
async fn reclaim_route(
    axum::extract::State(state): axum::extract::State<std::sync::Arc<DaemonState>>,
    axum::Json(body): axum::Json<ReclaimBody>,
) -> axum::response::Response {
    let repos = match resolve(&state, body.cwd).await {
        Ok(repos) => repos,
        Err((status, message)) => return crate::serve::err_response(status, &message),
    };
    match apply_all(&state, &repos).await {
        Ok(outcome) => {
            crate::host_pressure::request_resample();
            crate::serve::json_response(&outcome)
        }
        Err(error) => {
            crate::serve::err_response(axum::http::StatusCode::INTERNAL_SERVER_ERROR, &error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use std::sync::Arc;

    use crate::serve::DaemonState;

    fn git(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
    }

    /// A repo pushed to a bare `origin` with one merged, clean worktree: provably reclaimable.
    fn repo_with_stale_worktree(base: &Path, name: &str) -> (PathBuf, PathBuf) {
        let remote = base.join(format!("{name}-remote.git"));
        let repo = base.join(name);
        std::fs::create_dir_all(&remote).unwrap();
        std::fs::create_dir_all(&repo).unwrap();
        git(&remote, &["init", "-q", "--bare", "--initial-branch=main"]);
        git(&repo, &["init", "-q", "--initial-branch=main"]);
        std::fs::write(repo.join("README"), "x\n").unwrap();
        git(&repo, &["add", "README"]);
        git(&repo, &["commit", "-qm", "init", "--no-gpg-sign"]);
        git(
            &repo,
            &["remote", "add", "origin", &remote.to_string_lossy()],
        );
        git(&repo, &["push", "-q", "-u", "origin", "main"]);
        let wt = repo.join(".forge/worktrees/old");
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                &wt.to_string_lossy(),
                "-b",
                "forge/old",
                "HEAD",
            ],
        );
        (repo.canonicalize().unwrap(), wt)
    }

    fn state(default_cwd: &Path, roots: Vec<PathBuf>) -> Arc<DaemonState> {
        Arc::new(DaemonState {
            registry: Arc::new(crate::serve::SessionRegistry::new()),
            terminals: Arc::new(crate::serve_terminal::TerminalRegistry::new()),
            store: Arc::new(forge_store::Store::open_in_memory().unwrap()),
            base: "/tok".into(),
            mock: true,
            default_cwd: default_cwd.display().to_string(),
            project_roots: roots,
            push: None,
            apns: None,
            voice: crate::voice::VoiceState::new(),
            anywhere_enable: tokio::sync::watch::channel(false).0,
        })
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn reclaim_without_a_cwd_covers_every_known_repo_even_when_the_daemon_runs_outside_one() {
        use tower::ServiceExt;
        let base = tempfile::tempdir().unwrap();
        let home = base.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let (repo_a, wt_a) = repo_with_stale_worktree(base.path(), "a");
        let holder = base.path().join("holder");
        std::fs::create_dir_all(&holder).unwrap();
        let (repo_b, wt_b) = repo_with_stale_worktree(&holder, "b");
        let scratch = base.path().join("not-a-repo");
        std::fs::create_dir_all(&scratch).unwrap();
        let roots = vec![
            home.clone(),
            repo_a.clone(),
            repo_a.clone(),
            scratch,
            holder.canonicalize().unwrap(),
        ];
        let state = state(&home, roots);
        let router = routes("/tok").with_state(state);

        let preview = router
            .clone()
            .oneshot(
                axum::http::Request::get("/tok/api/host/pressure")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(preview.status(), axum::http::StatusCode::OK);
        let bytes = axum::body::to_bytes(preview.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let repos = body["repos"].as_array().unwrap();
        assert_eq!(repos.len(), 2, "deduped, non-repos skipped: {body}");
        assert!(body["reclaimable_bytes"].is_u64(), "{body}");

        let response = router
            .oneshot(
                axum::http::Request::post("/tok/api/host/reclaim")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let out: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(out["removed"], 2, "{out}");
        assert!(!wt_a.exists() && !wt_b.exists());
        assert!(repo_a.exists() && repo_b.exists());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn reclaim_without_a_cwd_and_no_known_repo_is_a_400() {
        use tower::ServiceExt;
        let base = tempfile::tempdir().unwrap();
        let state = state(base.path(), vec![base.path().to_path_buf()]);
        let response = routes("/tok")
            .with_state(state)
            .oneshot(
                axum::http::Request::post("/tok/api/host/reclaim")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    }
}
