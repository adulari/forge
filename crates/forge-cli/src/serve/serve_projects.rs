//! Safe project discovery and browsing for the Serve control surface.
//!
//! Passive browsing is restricted to configured canonical roots even though authenticated session
//! creation may still accept an explicit directory. This owner also projects durable and running
//! session workspaces into the recent-project catalog without exposing generated worktrees.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::response::Response;

use super::{err_response, json_response, DaemonState};

#[derive(Clone, Debug, serde::Serialize)]
struct ProjectRow {
    path: String,
    name: String,
    is_git_repo: bool,
    last_activity: Option<i64>,
}

#[derive(serde::Serialize)]
struct ProjectCatalog {
    default_cwd: String,
    recent: Vec<ProjectRow>,
    roots: Vec<ProjectRow>,
}

#[derive(serde::Deserialize)]
pub(super) struct BrowseProjectsQuery {
    path: Option<String>,
}

#[derive(Debug, serde::Serialize)]
struct BrowseProjectsResponse {
    path: String,
    parent: Option<String>,
    entries: Vec<ProjectRow>,
    roots: Vec<ProjectRow>,
    truncated: bool,
}

fn expand_project_root(raw: &str) -> PathBuf {
    if raw == "~" {
        return std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(raw));
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(raw)
}

pub(super) fn resolve_project_roots(default_cwd: &Path, configured: &[String]) -> Vec<PathBuf> {
    let mut roots = Vec::with_capacity(configured.len() + 1);
    for candidate in std::iter::once(default_cwd.to_path_buf())
        .chain(configured.iter().map(|raw| expand_project_root(raw)))
    {
        match candidate.canonicalize() {
            Ok(path) if path.is_dir() => {
                if !roots.contains(&path) {
                    roots.push(path);
                }
            }
            Ok(_) => eprintln!(
                "⚠ remote project root is not a directory: {}",
                candidate.display()
            ),
            Err(error) => eprintln!(
                "⚠ remote project root is unavailable ({}): {error}",
                candidate.display()
            ),
        }
    }
    roots
}

fn project_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| path.display().to_string())
}

fn project_row(path: &Path, last_activity: Option<i64>) -> ProjectRow {
    ProjectRow {
        path: path.display().to_string(),
        name: project_name(path),
        is_git_repo: path.join(".git").exists(),
        last_activity,
    }
}

fn path_is_browsable(path: &Path, roots: &[PathBuf]) -> bool {
    roots.iter().any(|root| path.starts_with(root))
}

fn browse_project_directory(
    requested: PathBuf,
    roots: Vec<PathBuf>,
) -> Result<BrowseProjectsResponse, String> {
    let path = requested
        .canonicalize()
        .map_err(|error| format!("project directory is unavailable: {error}"))?;
    if !path.is_dir() {
        return Err("project path is not a directory".to_string());
    }
    if !path_is_browsable(&path, &roots) {
        return Err("project path is outside the configured browse roots".to_string());
    }

    let mut entries = std::fs::read_dir(&path)
        .map_err(|error| format!("project directory cannot be read: {error}"))?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let file_type = entry.file_type().ok()?;
            if !file_type.is_dir() {
                return None;
            }
            let name = entry.file_name();
            if name.to_string_lossy().starts_with('.') {
                return None;
            }
            let canonical = entry.path().canonicalize().ok()?;
            path_is_browsable(&canonical, &roots).then(|| project_row(&canonical, None))
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        right
            .is_git_repo
            .cmp(&left.is_git_repo)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
    let truncated = entries.len() > 200;
    entries.truncate(200);
    let parent = path
        .parent()
        .filter(|parent| path_is_browsable(parent, &roots))
        .map(|parent| parent.display().to_string());
    let root_rows = roots.iter().map(|root| project_row(root, None)).collect();
    Ok(BrowseProjectsResponse {
        path: path.display().to_string(),
        parent,
        entries,
        roots: root_rows,
        truncated,
    })
}

pub(super) async fn project_catalog(State(state): State<Arc<DaemonState>>) -> Response {
    let mut candidates: Vec<(String, i64)> = state
        .registry
        .all()
        .await
        .into_iter()
        .filter(|handle| handle.worktree.is_none())
        .map(|handle| {
            (
                handle.cwd.clone(),
                handle
                    .last_activity
                    .load(std::sync::atomic::Ordering::Relaxed),
            )
        })
        .collect();

    let store = state.store.clone();
    let past = match tokio::task::spawn_blocking(move || store.list_sessions_for_resume()).await {
        Ok(Ok(rows)) => rows,
        Ok(Err(error)) => {
            return err_response(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                &format!("listing recent projects failed: {error}"),
            )
        }
        Err(error) => {
            return err_response(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                &format!("listing recent projects task failed: {error}"),
            )
        }
    };
    candidates.extend(
        past.into_iter()
            .filter(|session| session.worktree_path.is_none())
            .map(|session| (session.cwd, session.last_activity)),
    );
    candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.1));

    let default_path = PathBuf::from(&state.default_cwd);
    let mut seen = HashSet::new();
    seen.insert(default_path);
    let recent = candidates
        .into_iter()
        .filter_map(|(raw, last_activity)| {
            let path = PathBuf::from(raw).canonicalize().ok()?;
            if !path.is_dir() || !seen.insert(path.clone()) {
                return None;
            }
            Some(project_row(&path, Some(last_activity)))
        })
        .take(8)
        .collect();
    let roots = state
        .project_roots
        .iter()
        .map(|root| project_row(root, None))
        .collect();

    json_response(&ProjectCatalog {
        default_cwd: state.default_cwd.clone(),
        recent,
        roots,
    })
}

fn resolve_browse_request(
    path: Option<String>,
    roots: &[PathBuf],
) -> Option<Result<PathBuf, String>> {
    match path.filter(|path| !path.trim().is_empty()) {
        Some(path) => {
            let path = PathBuf::from(path);
            Some(if path.is_absolute() {
                Ok(path)
            } else {
                Err("project browse path must be absolute".to_string())
            })
        }
        None => roots.first().cloned().map(Ok),
    }
}

pub(super) async fn browse_projects(
    State(state): State<Arc<DaemonState>>,
    Query(query): Query<BrowseProjectsQuery>,
) -> Response {
    let roots = state.project_roots.clone();
    let requested = resolve_browse_request(query.path, &roots);
    let Some(requested) = requested else {
        return err_response(
            axum::http::StatusCode::NOT_FOUND,
            "no project roots are available",
        );
    };

    let requested = match requested {
        Ok(requested) => requested,
        Err(message) => return err_response(axum::http::StatusCode::BAD_REQUEST, &message),
    };
    let result =
        tokio::task::spawn_blocking(move || browse_project_directory(requested, roots)).await;
    match result {
        Ok(Ok(response)) => json_response(&response),
        Ok(Err(message)) => err_response(axum::http::StatusCode::BAD_REQUEST, &message),
        Err(error) => err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            &format!("browsing project directory failed: {error}"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{browse_project_directory, resolve_browse_request, resolve_project_roots};

    #[test]
    fn roots_are_canonical_deduplicated_and_default_first() {
        let temp = tempfile::tempdir().unwrap();
        let default = temp.path().join("default");
        let extra = temp.path().join("extra");
        std::fs::create_dir_all(&default).unwrap();
        std::fs::create_dir_all(&extra).unwrap();
        let roots = resolve_project_roots(
            &default,
            &[
                default.join(".").display().to_string(),
                extra.display().to_string(),
                extra.join("..").join("extra").display().to_string(),
            ],
        );
        assert_eq!(
            roots,
            [
                default.canonicalize().unwrap(),
                extra.canonicalize().unwrap()
            ]
        );
    }

    #[test]
    fn browse_requests_default_to_the_first_root_and_reject_relative_paths() {
        let roots = vec![PathBuf::from("/first"), PathBuf::from("/second")];
        assert_eq!(
            resolve_browse_request(None, &roots),
            Some(Ok(roots[0].clone()))
        );
        assert_eq!(
            resolve_browse_request(Some(String::new()), &roots),
            Some(Ok(roots[0].clone()))
        );
        assert!(matches!(
            resolve_browse_request(Some("relative/project".into()), &roots),
            Some(Err(message)) if message.contains("must be absolute")
        ));
    }

    #[test]
    fn browser_stays_inside_roots_and_prioritizes_git_projects() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let ordinary = root.path().join("alpha");
        let git = root.path().join("zeta-git");
        std::fs::create_dir_all(&ordinary).unwrap();
        std::fs::create_dir_all(git.join(".git")).unwrap();
        std::fs::create_dir_all(root.path().join(".hidden")).unwrap();
        let canonical_root = root.path().canonicalize().unwrap();
        let result =
            browse_project_directory(canonical_root.clone(), vec![canonical_root.clone()]).unwrap();
        assert!(result.parent.is_none());
        assert_eq!(
            result
                .entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            ["zeta-git", "alpha"]
        );
        assert!(result.entries[0].is_git_repo);
        let error = browse_project_directory(outside.path().to_path_buf(), vec![canonical_root])
            .unwrap_err();
        assert!(
            error.contains("outside the configured browse roots"),
            "{error}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn browser_rejects_symlink_escapes() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let escape = root.path().join("escape");
        symlink(outside.path(), &escape).unwrap();
        let canonical_root = root.path().canonicalize().unwrap();
        let listing =
            browse_project_directory(canonical_root.clone(), vec![canonical_root.clone()]).unwrap();
        assert!(listing.entries.iter().all(|entry| entry.name != "escape"));
        let error = browse_project_directory(escape, vec![canonical_root]).unwrap_err();
        assert!(
            error.contains("outside the configured browse roots"),
            "{error}"
        );
    }
}
