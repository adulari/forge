//! Stage, unstage and commit routes of the review dock.

use super::*;

/// Stage or unstage `paths`, touching only the index.
pub(super) fn update_index(
    root: &std::path::Path,
    paths: &[String],
    unstage: bool,
) -> Result<(), String> {
    let mut args: Vec<&str> = if !unstage {
        vec!["add", "--"]
    } else if git_raw(root, &["rev-parse", "--verify", "-q", "HEAD"]).is_ok() {
        // `restore --staged` (not `reset`) so an unstage never moves HEAD or touches the
        // working tree — the dock only ever changes what is in the index.
        vec!["restore", "--staged", "--"]
    } else {
        // No commit yet, so nothing to restore from: unstaging means dropping the paths from
        // the index again, leaving the files on disk.
        vec!["rm", "--cached", "-r", "-q", "--ignore-unmatch", "--"]
    };
    args.extend(paths.iter().map(String::as_str));
    git_raw(root, &args).map(|_| ())
}

async fn stage_impl(state: Arc<DaemonState>, request: GitPathsRequest, unstage: bool) -> Response {
    let Some(dir) = session_dir(&state, &request.session).await else {
        return no_such_session();
    };
    let paths = match repo_relative_all(&request.paths) {
        Ok(paths) => paths,
        Err(message) => return err_response(axum::http::StatusCode::BAD_REQUEST, &message),
    };
    let result = tokio::task::spawn_blocking(move || -> Result<(), String> {
        let root = repo_root_of(&dir)?;
        update_index(&root, &paths, unstage)
    })
    .await;
    match result {
        Ok(Ok(())) => json_response(&serde_json::json!({ "ok": true })),
        Ok(Err(message)) => err_response(axum::http::StatusCode::BAD_REQUEST, &message),
        Err(_) => err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "git index update failed",
        ),
    }
}

/// `POST /api/git/stage` `{session, paths[]}`.
pub(crate) async fn git_stage(
    State(state): State<Arc<DaemonState>>,
    Json(request): Json<GitPathsRequest>,
) -> Response {
    stage_impl(state, request, false).await
}

/// `POST /api/git/unstage` `{session, paths[]}`.
pub(crate) async fn git_unstage(
    State(state): State<Arc<DaemonState>>,
    Json(request): Json<GitPathsRequest>,
) -> Response {
    stage_impl(state, request, true).await
}

/// `POST /api/git/commit` `{session, message}` — commits ONLY what is already staged (no `-a`), so
/// the dock's staging decisions are the whole contract.
pub(crate) async fn git_commit(
    State(state): State<Arc<DaemonState>>,
    Json(request): Json<GitCommitRequest>,
) -> Response {
    let Some(dir) = session_dir(&state, &request.session).await else {
        return no_such_session();
    };
    let message = request.message.trim().to_string();
    if message.is_empty() {
        return err_response(
            axum::http::StatusCode::BAD_REQUEST,
            "a commit message is required",
        );
    }
    let result = tokio::task::spawn_blocking(move || -> Result<GitCommitResponse, String> {
        let root = repo_root_of(&dir)?;
        if git_stdout(&root, &["diff", "--cached", "--name-only"])
            .unwrap_or_default()
            .is_empty()
        {
            return Err("nothing staged to commit".to_string());
        }
        // `--` terminates options so a message beginning with `-` is never read as one; `-F -`
        // is avoided because it would need stdin plumbing for no gain.
        git_raw(&root, &["commit", "-m", message.as_str(), "--"])?;
        let sha = git_stdout(&root, &["rev-parse", "HEAD"]).unwrap_or_default();
        let summary = git_stdout(&root, &["show", "-s", "--format=%s", "HEAD"]).unwrap_or_default();
        Ok(GitCommitResponse {
            ok: true,
            sha,
            summary,
        })
    })
    .await;
    match result {
        Ok(Ok(response)) => json_response(&response),
        Ok(Err(message)) => err_response(axum::http::StatusCode::BAD_REQUEST, &message),
        Err(_) => err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "git commit failed",
        ),
    }
}
