//! Project onboarding and worktree routes: clone a repository, `git init` a folder, the Home
//! scope, the signed-in `gh` repository list, worktrees of a project, a stash-aware pull, and
//! rename / relocate / hide / repair for the project catalog.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::{Json, Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::{get, patch, post};
use axum::Router;
use forge_core::project_registry::{ProjectPatch, ProjectRegistry};
use serde::Deserialize;

use crate::serve::{err_response, json_response, DaemonState};

pub(crate) fn routes(base: &str) -> Router<Arc<DaemonState>> {
    Router::new()
        .route(&format!("{base}/api/projects/clone"), post(clone))
        .route(&format!("{base}/api/projects/init"), post(init))
        .route(&format!("{base}/api/projects/home"), get(home))
        .route(&format!("{base}/api/projects/gh-repos"), get(gh_repos))
        .route(&format!("{base}/api/projects/entry"), patch(update_entry))
        .route(&format!("{base}/api/projects/repair"), post(repair))
        .route(&format!("{base}/api/worktrees"), get(list_worktrees))
        .route(&format!("{base}/api/worktrees/pull"), post(pull))
}

fn blocking_failed(error: impl std::fmt::Display) -> Response {
    err_response(
        StatusCode::INTERNAL_SERVER_ERROR,
        &format!("background task failed: {error}"),
    )
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

#[derive(Deserialize)]
struct CloneReq {
    url: String,
    /// Folder name under `parent`; derived from the URL when absent.
    name: Option<String>,
    /// Where to clone into: an existing directory. Defaults to `~/Projects`.
    parent: Option<String>,
}

async fn clone(State(_state): State<Arc<DaemonState>>, Json(req): Json<CloneReq>) -> Response {
    let parent = match req
        .parent
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        Some(parent) => PathBuf::from(parent),
        None => match home_dir() {
            Some(home) => home.join("Projects"),
            None => return err_response(StatusCode::BAD_REQUEST, "no home directory"),
        },
    };
    if !parent.is_absolute() {
        return err_response(StatusCode::BAD_REQUEST, "parent must be an absolute path");
    }
    let result = tokio::task::spawn_blocking(move || {
        std::fs::create_dir_all(&parent)
            .map_err(|e| format!("creating {}: {e}", parent.display()))?;
        forge_core::project_ops::clone_repo(&req.url, &parent, req.name.as_deref())
    })
    .await;
    match result {
        Ok(Ok(path)) => json_response(&serde_json::json!({
            "path": path.display().to_string(),
            "name": path.file_name().and_then(|n| n.to_str()).unwrap_or_default(),
        })),
        Ok(Err(message)) => err_response(StatusCode::BAD_REQUEST, &message),
        Err(error) => blocking_failed(error),
    }
}

#[derive(Deserialize)]
struct InitReq {
    path: String,
}

async fn init(State(_state): State<Arc<DaemonState>>, Json(req): Json<InitReq>) -> Response {
    let path = PathBuf::from(req.path.trim());
    if !path.is_absolute() {
        return err_response(StatusCode::BAD_REQUEST, "path must be absolute");
    }
    let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
    if canonical.parent().is_none()
        || home_dir().and_then(|h| h.canonicalize().ok()) == Some(canonical.clone())
    {
        return err_response(
            StatusCode::BAD_REQUEST,
            "refusing to create a repository in a filesystem root or your home folder",
        );
    }
    match tokio::task::spawn_blocking(move || forge_core::project_ops::init_repo(&canonical)).await
    {
        Ok(Ok(())) => json_response(
            &serde_json::json!({ "path": path.display().to_string(), "is_git_repo": true }),
        ),
        Ok(Err(message)) => err_response(StatusCode::BAD_REQUEST, &message),
        Err(error) => blocking_failed(error),
    }
}

async fn home() -> Response {
    match forge_core::project_ops::home_dir() {
        Ok(path) => json_response(&serde_json::json!({
            "path": path.display().to_string(),
            "name": "Home",
        })),
        Err(message) => err_response(StatusCode::INTERNAL_SERVER_ERROR, &message),
    }
}

async fn gh_repos() -> Response {
    match tokio::task::spawn_blocking(forge_core::project_ops::gh_repos).await {
        Ok(repos) => json_response(&repos),
        Err(error) => blocking_failed(error),
    }
}

#[derive(Deserialize)]
struct EntryReq {
    path: String,
    #[serde(flatten)]
    patch: PatchFields,
}

#[derive(Deserialize, Default)]
struct PatchFields {
    name: Option<String>,
    disabled: Option<bool>,
    relocate_to: Option<String>,
}

async fn update_entry(Json(req): Json<EntryReq>) -> Response {
    let patch = ProjectPatch {
        name: req.patch.name,
        disabled: req.patch.disabled,
        relocate_to: req.patch.relocate_to,
    };
    let result = tokio::task::spawn_blocking(move || {
        let mut registry = ProjectRegistry::load();
        registry.apply(&req.path, &patch)?;
        registry.save()
    })
    .await;
    match result {
        Ok(Ok(())) => json_response(&serde_json::json!({ "ok": true })),
        Ok(Err(message)) => err_response(StatusCode::BAD_REQUEST, &message),
        Err(error) => blocking_failed(error),
    }
}

/// Fold catalog rows that are the same repository (a subfolder of it, a symlink to it, a second
/// path recorded before a move) into one. Returns the groups that were merged.
async fn repair(State(state): State<Arc<DaemonState>>) -> Response {
    let paths = crate::serve::project_catalog_paths(&state).await;
    let result = tokio::task::spawn_blocking(move || {
        let mut registry = ProjectRegistry::load();
        let groups = duplicate_groups(&paths);
        for (keep, dups) in &groups {
            registry.merge_into(keep, dups);
        }
        registry.save()?;
        Ok::<_, String>(groups)
    })
    .await;
    match result {
        Ok(Ok(groups)) => json_response(&serde_json::json!({
            "merged": groups.iter().map(|(_, d)| d.len()).sum::<usize>(),
            "groups": groups.iter().map(|(k, d)| serde_json::json!({ "keep": k, "duplicates": d })).collect::<Vec<_>>(),
        })),
        Ok(Err(message)) => err_response(StatusCode::INTERNAL_SERVER_ERROR, &message),
        Err(error) => blocking_failed(error),
    }
}

/// Paths grouped by the repository top they belong to; only groups with more than one distinct
/// path are returned. The kept path is the repository top itself (whichever recorded path names
/// it, else the top), never a subfolder that merely was opened most recently: every duplicate is
/// redirected to it, so a subfolder would become the project root.
pub(crate) fn duplicate_groups(paths: &[String]) -> Vec<(String, Vec<String>)> {
    let mut members: Vec<(String, Vec<String>)> = Vec::new();
    for path in paths {
        let key = repo_key(Path::new(path));
        match members.iter_mut().find(|(k, _)| *k == key) {
            Some((_, all)) => all.push(path.clone()),
            None => members.push((key, vec![path.clone()])),
        }
    }
    members
        .into_iter()
        .filter(|(_, all)| all.len() > 1)
        .map(|(key, all)| {
            let keep = all
                .iter()
                .find(|p| {
                    Path::new(p)
                        .canonicalize()
                        .is_ok_and(|c| c == Path::new(&key))
                })
                .cloned()
                .unwrap_or_else(|| key.clone());
            let dups = all.into_iter().filter(|p| *p != keep).collect();
            (keep, dups)
        })
        .collect()
}

fn repo_key(path: &Path) -> String {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    std::process::Command::new("git")
        .arg("-C")
        .arg(&canonical)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|top| !top.is_empty())
        .unwrap_or_else(|| canonical.display().to_string())
}

#[derive(Deserialize)]
struct CwdQuery {
    cwd: String,
}

async fn list_worktrees(Query(query): Query<CwdQuery>) -> Response {
    let result = tokio::task::spawn_blocking(move || {
        let entries = forge_core::worktree_ops::list(Path::new(&query.cwd))?;
        Ok::<_, String>(
            entries
                .into_iter()
                .map(|e| {
                    let dirty = std::process::Command::new("git")
                        .arg("-C")
                        .arg(&e.path)
                        .args(["status", "--porcelain"])
                        .output()
                        .map(|o| !o.stdout.is_empty())
                        .unwrap_or(false);
                    serde_json::json!({
                        "path": e.path.display().to_string(),
                        "branch": e.branch,
                        "head": e.head,
                        "main": e.main,
                        "dirty": dirty,
                    })
                })
                .collect::<Vec<_>>(),
        )
    })
    .await;
    match result {
        Ok(Ok(rows)) => json_response(&serde_json::json!({ "worktrees": rows })),
        Ok(Err(message)) => err_response(StatusCode::BAD_REQUEST, &message),
        Err(error) => blocking_failed(error),
    }
}

#[derive(Deserialize)]
struct PullReq {
    cwd: String,
}

async fn pull(State(state): State<Arc<DaemonState>>, Json(req): Json<PullReq>) -> Response {
    let dir = PathBuf::from(&req.cwd);
    if !dir.is_dir() {
        return err_response(StatusCode::BAD_REQUEST, "cwd is not a directory");
    }
    // The pull stashes every uncommitted file in the checkout; under a working agent that pulls
    // its in-progress edits out from beneath it.
    let target = dir.canonicalize().unwrap_or_else(|_| dir.clone());
    let working = state.registry.all().await.into_iter().any(|handle| {
        let root = handle
            .worktree
            .clone()
            .unwrap_or_else(|| handle.cwd.clone());
        let root = PathBuf::from(root);
        handle.snapshot_rx.borrow().snapshot.busy
            && root.canonicalize().unwrap_or(root).starts_with(&target)
    });
    if working {
        return err_response(
            StatusCode::CONFLICT,
            "a session is working in this checkout; pull when its turn has finished",
        );
    }
    match tokio::task::spawn_blocking(move || forge_core::project_ops::pull(&dir)).await {
        Ok(outcome) => json_response(&outcome),
        Err(error) => blocking_failed(error),
    }
}

#[cfg(test)]
#[path = "serve_project_ops/tests.rs"]
mod tests;
