//! `POST /api/git/hunk` — stage, unstage or revert ONE hunk of a file.
//!
//! The client sends the hunk it is looking at as a single-file, single-hunk unified diff (the
//! `--- a/<path>` / `+++ b/<path>` header plus one `@@` block, as `GET /api/git/diff` returned it).
//! The daemon never trusts that patch beyond what [`validate_hunk_patch`] proves — it must touch
//! exactly the named path and carry exactly the named hunk — and then hands it to `git apply`:
//!
//! - `stage`   `git apply --cached`      (index ← the hunk; patch is a working-tree-vs-index hunk)
//! - `unstage` `git apply -R --cached`   (index → the hunk undone; patch is an index-vs-HEAD hunk)
//! - `revert`  `git apply -R`            (working tree → the hunk undone; patch is a
//!   working-tree-vs-index hunk). Working tree only: reverting a hunk that is already staged is
//!   `unstage` first, then `revert`.
//!
//! A patch that no longer applies (the file changed since the client fetched the diff) is a 409
//! with code `stale_patch`, so the client refetches instead of retrying. `git apply` is all or
//! nothing, so a refused hunk changes nothing.

use std::io::Write as _;
use std::process::{Command, Stdio};

use super::branches::session_git_context;
use super::*;
use crate::serve::err_response_coded;

const MAX_PATCH_BYTES: usize = 1024 * 1024;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GitHunkRequest {
    session: String,
    path: String,
    /// The hunk's `@@ -a,b +c,d @@ ...` line, exactly as the diff route returned it.
    hunk_header: String,
    patch: String,
    action: HunkAction,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HunkAction {
    Stage,
    Unstage,
    Revert,
}

#[derive(serde::Serialize)]
struct GitHunkResponse {
    ok: bool,
    action: HunkAction,
}

/// Check that `patch` is one hunk of `rel` and nothing else; return it with a trailing newline.
///
/// A bare `@@` block (no file header) is accepted and given the plain modify header.
pub(crate) fn validate_hunk_patch(
    rel: &str,
    hunk_header: &str,
    patch: &str,
) -> Result<String, String> {
    if patch.len() > MAX_PATCH_BYTES {
        return Err("patch is too large".to_string());
    }
    if patch.contains('\0') {
        return Err("patch must be text".to_string());
    }
    let header = hunk_header.trim_end();
    if !header.starts_with("@@ -") {
        return Err("hunk_header is not a unified-diff hunk header".to_string());
    }
    let a_name = format!("a/{rel}");
    let b_name = format!("b/{rel}");
    let mut seen_old = false;
    let mut seen_new = false;
    let mut in_body = false;
    let mut out = String::new();
    if patch.starts_with("@@ ") {
        out.push_str(&format!("--- {a_name}\n+++ {b_name}\n"));
        seen_old = true;
        seen_new = true;
    }
    // Split on '\n' only: `str::lines` would also drop the '\r' of a CRLF file's lines, and a
    // body line without it no longer matches the file, so git refuses the patch.
    let mut raw_lines: Vec<&str> = patch.split('\n').collect();
    if raw_lines.last() == Some(&"") {
        raw_lines.pop();
    }
    for raw in raw_lines {
        let line = raw.trim_end_matches('\r');
        if line.starts_with("@@") {
            if in_body {
                return Err("patch must contain exactly one hunk".to_string());
            }
            if line.trim_end() != header {
                return Err("patch hunk does not match hunk_header".to_string());
            }
            if !seen_old || !seen_new {
                return Err("patch is missing its ---/+++ file header".to_string());
            }
            in_body = true;
        } else if in_body {
            // `\ No newline at end of file` and the three gutter characters; an empty line is a
            // context line whose trailing space a transport stripped.
            if !(line.is_empty() || line.starts_with([' ', '+', '-', '\\'])) {
                return Err("patch body has a line outside the unified-diff format".to_string());
            }
        } else if let Some(old) = line.strip_prefix("--- ") {
            if seen_old || (old != a_name && old != "/dev/null") {
                return Err("patch must change only the named path".to_string());
            }
            seen_old = true;
        } else if let Some(new) = line.strip_prefix("+++ ") {
            if !seen_old || seen_new || (new != b_name && new != "/dev/null") {
                return Err("patch must change only the named path".to_string());
            }
            seen_new = true;
        } else if line == format!("diff --git {a_name} {b_name}")
            || line.starts_with("index ")
            || line.starts_with("new file mode 100")
            || line.starts_with("deleted file mode 100")
        {
            if seen_old {
                return Err("patch header lines are out of order".to_string());
            }
        } else {
            // Renames, copies, mode changes, symlinks (mode 120000) and binary patches would all
            // let one "hunk" do more than edit lines of the named file.
            return Err(format!("unsupported patch header line: {line}"));
        }
        out.push_str(if in_body && !line.starts_with("@@") {
            raw
        } else {
            line
        });
        out.push('\n');
    }
    if !in_body {
        return Err("patch contains no hunk".to_string());
    }
    Ok(out)
}

fn git_apply(root: &Path, flags: &[&str], patch: &str) -> Result<(), (bool, String)> {
    let mut command = Command::new("git");
    command
        .args(["--literal-pathspecs", "-C", root.to_str().unwrap_or(".")])
        .arg("apply")
        .args(flags)
        .args(["--whitespace=nowarn", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|e| (false, e.to_string()))?;
    if let Some(mut stdin) = child.stdin.take() {
        // A write error means git already rejected the patch; its stderr says why.
        let _ = stdin.write_all(patch.as_bytes());
    }
    let output = child
        .wait_with_output()
        .map_err(|e| (false, e.to_string()))?;
    if output.status.success() {
        return Ok(());
    }
    let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let malformed = message.contains("corrupt patch") || message.contains("unrecognized input");
    Err((!malformed, message))
}

pub(crate) fn apply_hunk(
    root: &Path,
    action: HunkAction,
    patch: &str,
) -> Result<(), (bool, String)> {
    let flags: &[&str] = match action {
        HunkAction::Stage => &["--cached"],
        HunkAction::Unstage => &["-R", "--cached"],
        HunkAction::Revert => &["-R"],
    };
    git_apply(root, flags, patch)
}

/// `POST /api/git/hunk` `{session, path, hunk_header, patch, action}`.
pub(crate) async fn git_hunk(
    State(state): State<Arc<DaemonState>>,
    Json(request): Json<GitHunkRequest>,
) -> Response {
    use axum::http::StatusCode;
    let Some(dir) = session_dir(&state, &request.session).await else {
        return no_such_session();
    };
    let rel = match repo_relative(&request.path) {
        Ok(rel) => rel,
        Err(message) => return err_response(StatusCode::BAD_REQUEST, &message),
    };
    let patch = match validate_hunk_patch(&rel, &request.hunk_header, &request.patch) {
        Ok(patch) => patch,
        Err(message) => return err_response(StatusCode::BAD_REQUEST, &message),
    };
    if request.action == HunkAction::Revert
        && session_git_context(&state, &request.session)
            .await
            .is_some_and(|context| context.busy)
    {
        return err_response_coded(
            StatusCode::CONFLICT,
            "session_busy",
            "the session is mid-turn; wait for it to finish before reverting a hunk",
        );
    }
    let action = request.action;
    let result = tokio::task::spawn_blocking(move || {
        let root = repo_root_of(&dir).map_err(|message| (false, message))?;
        apply_hunk(&root, action, &patch)
    })
    .await;
    match result {
        Ok(Ok(())) => json_response(&GitHunkResponse { ok: true, action }),
        Ok(Err((true, message))) => err_response_coded(
            StatusCode::CONFLICT,
            "stale_patch",
            &format!("the hunk no longer applies; refetch the diff ({message})"),
        ),
        Ok(Err((false, message))) => err_response(StatusCode::BAD_REQUEST, &message),
        Err(_) => err_response(StatusCode::INTERNAL_SERVER_ERROR, "git apply failed"),
    }
}

#[cfg(test)]
mod tests;
