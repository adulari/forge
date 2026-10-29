//! `POST /api/workspace/{create,rename,delete}` — file and folder management for the files dock.
//!
//! Same confinement as the editor: the root comes from the live session, every path is relative
//! and normalised (no `..`, no `.git`), and the deepest existing ancestor of a new path must
//! canonicalise inside the root, so a symlinked directory cannot carry an operation outside it.
//! Nothing here overwrites: create and rename answer 409 when the destination exists.

use axum::routing::post;
use axum::Router;

use super::*;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkspaceCreateRequest {
    session: String,
    path: String,
    /// `file` (default) or `directory`.
    #[serde(default)]
    kind: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkspaceRenameRequest {
    session: String,
    from: String,
    to: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkspaceDeleteRequest {
    session: String,
    path: String,
}

#[derive(Debug, serde::Serialize)]
pub(crate) struct WorkspaceEditResponse {
    ok: bool,
    path: String,
    kind: &'static str,
}

pub(crate) fn routes(base: &str) -> Router<Arc<DaemonState>> {
    Router::new()
        .route(
            &format!("{base}/api/workspace/create"),
            post(workspace_create),
        )
        .route(
            &format!("{base}/api/workspace/rename"),
            post(workspace_rename),
        )
        .route(
            &format!("{base}/api/workspace/delete"),
            post(workspace_delete),
        )
}

/// Make sure the deepest ancestor of `target` that already exists resolves inside `root`, then
/// create the missing directories above `target`.
fn prepare_parent(root: &Path, target: &Path) -> Result<(), WorkspaceFailure> {
    let mut ancestor = target.parent();
    while let Some(candidate) = ancestor {
        match fs::symlink_metadata(candidate) {
            Ok(_) => {
                let resolved = fs::canonicalize(candidate)?;
                if !resolved.starts_with(root) {
                    return Err(WorkspaceFailure::forbidden(
                        "workspace symlink points outside the session root",
                    ));
                }
                if !resolved.is_dir() {
                    return Err(WorkspaceFailure::bad("a parent of that path is a file"));
                }
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                ancestor = candidate.parent();
            }
            Err(error) => return Err(error.into()),
        }
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}

fn exists_failure(relative: &str) -> WorkspaceFailure {
    WorkspaceFailure::new(StatusCode::CONFLICT, format!("{relative} already exists"))
}

pub(super) fn create_entry(
    root: &Path,
    raw_path: &str,
    kind: Option<&str>,
) -> Result<WorkspaceEditResponse, WorkspaceFailure> {
    let directory = match kind.map(str::trim) {
        None | Some("") | Some("file") => false,
        Some("directory") => true,
        Some(_) => return Err(WorkspaceFailure::bad("kind must be file or directory")),
    };
    let relative = normalized_relative(raw_path, false)?;
    let wire = wire_path(&relative);
    let target = root.join(&relative);
    if fs::symlink_metadata(&target).is_ok() {
        return Err(exists_failure(&wire));
    }
    prepare_parent(root, &target)?;
    let created = if directory {
        fs::create_dir(&target)
    } else {
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .map(|_| ())
    };
    match created {
        Ok(()) => Ok(WorkspaceEditResponse {
            ok: true,
            path: wire,
            kind: if directory { "directory" } else { "file" },
        }),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            Err(exists_failure(&wire))
        }
        Err(error) => Err(error.into()),
    }
}

pub(super) fn rename_entry(
    root: &Path,
    raw_from: &str,
    raw_to: &str,
) -> Result<WorkspaceEditResponse, WorkspaceFailure> {
    let from = normalized_relative(raw_from, false)?;
    let to = normalized_relative(raw_to, false)?;
    let source = root.join(&from);
    let destination = root.join(&to);
    let metadata = fs::symlink_metadata(&source)?;
    if let Some(parent) = source.parent() {
        let resolved = fs::canonicalize(parent)?;
        if !resolved.starts_with(root) {
            return Err(WorkspaceFailure::forbidden(
                "workspace symlink points outside the session root",
            ));
        }
    }
    if from == to {
        return Err(WorkspaceFailure::bad(
            "the new path is the same as the old one",
        ));
    }
    if destination.starts_with(&source) {
        return Err(WorkspaceFailure::bad(
            "a folder cannot be moved into itself",
        ));
    }
    if fs::symlink_metadata(&destination).is_ok() {
        return Err(exists_failure(&wire_path(&to)));
    }
    prepare_parent(root, &destination)?;
    fs::rename(&source, &destination)?;
    Ok(WorkspaceEditResponse {
        ok: true,
        path: wire_path(&to),
        kind: if metadata.is_dir() {
            "directory"
        } else {
            "file"
        },
    })
}

pub(super) fn delete_entry(
    root: &Path,
    raw_path: &str,
) -> Result<WorkspaceEditResponse, WorkspaceFailure> {
    let relative = normalized_relative(raw_path, false)?;
    let target = root.join(&relative);
    let metadata = fs::symlink_metadata(&target)?;
    if let Some(parent) = target.parent() {
        let resolved = fs::canonicalize(parent)?;
        if !resolved.starts_with(root) {
            return Err(WorkspaceFailure::forbidden(
                "workspace symlink points outside the session root",
            ));
        }
    }
    let kind = if metadata.is_dir() {
        "directory"
    } else {
        "file"
    };
    if metadata.is_dir() {
        if target.join(".git").exists() {
            return Err(WorkspaceFailure::forbidden(
                "that folder is a separate git repository; delete it from a terminal",
            ));
        }
        fs::remove_dir_all(&target)?;
    } else {
        // A symlink is removed as the link, never followed.
        fs::remove_file(&target)?;
    }
    Ok(WorkspaceEditResponse {
        ok: true,
        path: wire_path(&relative),
        kind,
    })
}

async fn run_edit<F>(state: &DaemonState, session: &str, edit: F) -> Response
where
    F: FnOnce(&Path) -> Result<WorkspaceEditResponse, WorkspaceFailure> + Send + 'static,
{
    let root = match session_root(state, session).await {
        Ok(root) => root,
        Err(error) => return failure_response(error),
    };
    match tokio::task::spawn_blocking(move || edit(&root)).await {
        Ok(Ok(response)) => json_response(&response),
        Ok(Err(error)) => failure_response(error),
        Err(error) => failure_response(WorkspaceFailure::internal(error)),
    }
}

async fn workspace_create(
    State(state): State<Arc<DaemonState>>,
    Json(request): Json<WorkspaceCreateRequest>,
) -> Response {
    let WorkspaceCreateRequest {
        session,
        path,
        kind,
    } = request;
    run_edit(&state, &session, move |root| {
        create_entry(root, &path, kind.as_deref())
    })
    .await
}

async fn workspace_rename(
    State(state): State<Arc<DaemonState>>,
    Json(request): Json<WorkspaceRenameRequest>,
) -> Response {
    let WorkspaceRenameRequest { session, from, to } = request;
    run_edit(&state, &session, move |root| rename_entry(root, &from, &to)).await
}

async fn workspace_delete(
    State(state): State<Arc<DaemonState>>,
    Json(request): Json<WorkspaceDeleteRequest>,
) -> Response {
    let WorkspaceDeleteRequest { session, path } = request;
    run_edit(&state, &session, move |root| delete_entry(root, &path)).await
}

#[cfg(test)]
mod tests;
