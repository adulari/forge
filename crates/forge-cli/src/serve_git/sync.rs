//! `POST /api/git/{fetch,pull,push,discard}` — the dock's remote and destructive actions.
//!
//! Remote commands never force, never prompt (no TTY: a credential prompt would hang the
//! request), run under a timeout, and only accept a remote the repository already has or a URL
//! that passes [`validate_remote_url`](super::url::validate_remote_url). `pull` defaults to
//! `--ff-only` so it can never leave a half-merged tree behind unasked; `pull` and `discard`
//! refuse while the session is mid-turn, the same guard branch switching uses.

use std::time::Duration;

use super::branches::session_git_context;
use super::url::{validate_remote, NETWORK_ENV};
use super::*;

const NETWORK_TIMEOUT: Duration = Duration::from_secs(120);
const OUTPUT_TAIL_CHARS: usize = 2_000;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GitSyncRequest {
    session: String,
    #[serde(default)]
    remote: Option<String>,
    /// `pull` only: `ff` (default, `--ff-only`), `merge` or `rebase`.
    #[serde(default)]
    strategy: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GitDiscardRequest {
    session: String,
    paths: Vec<String>,
}

#[derive(serde::Serialize)]
pub(crate) struct GitSyncResponse {
    ok: bool,
    remote: Option<String>,
    branch: Option<String>,
    /// The tail of what git printed (progress goes to stderr, so both streams are included).
    output: String,
}

#[derive(serde::Serialize)]
pub(crate) struct GitDiscardResponse {
    ok: bool,
    discarded: usize,
}

struct SyncFailure {
    status: axum::http::StatusCode,
    message: String,
}

impl SyncFailure {
    fn bad(message: impl Into<String>) -> Self {
        Self {
            status: axum::http::StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }
}

fn tail(text: &str) -> String {
    let trimmed = text.trim();
    let count = trimmed.chars().count();
    if count <= OUTPUT_TAIL_CHARS {
        return trimmed.to_string();
    }
    trimmed.chars().skip(count - OUTPUT_TAIL_CHARS).collect()
}

/// Run `git -C <root> <args>` for a command that talks to a remote. Returns stdout + stderr.
async fn run_network_git(root: &Path, args: &[String]) -> Result<String, String> {
    let mut command = tokio::process::Command::new("git");
    command
        .arg("-C")
        .arg(root)
        .args([
            "-c",
            "protocol.ext.allow=never",
            "-c",
            "protocol.fd.allow=never",
        ])
        .args(args)
        .envs(NETWORK_ENV)
        .env("GIT_EDITOR", "true")
        .env("GIT_MERGE_AUTOEDIT", "no")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    if std::env::var_os("GIT_SSH_COMMAND").is_none() && std::env::var_os("GIT_SSH").is_none() {
        command.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes");
    }
    let output = match tokio::time::timeout(NETWORK_TIMEOUT, command.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => return Err(error.to_string()),
        Err(_) => {
            return Err(format!(
                "git {} timed out after {}s",
                args.first().map(String::as_str).unwrap_or("command"),
                NETWORK_TIMEOUT.as_secs()
            ))
        }
    };
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    if output.status.success() {
        Ok(tail(&text))
    } else {
        Err(tail(&text))
    }
}

fn current_branch(root: &Path) -> Result<String, SyncFailure> {
    git_stdout(root, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .ok()
        .filter(|branch| !branch.is_empty())
        .ok_or_else(|| SyncFailure::bad("HEAD is detached; switch to a branch first"))
}

fn configured_remotes(root: &Path) -> Vec<String> {
    git_stdout(root, &["remote"])
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect()
}

/// The remote a request addresses: the one it names (validated), else the branch's configured
/// remote, else `origin`, else the only remote there is.
fn resolve_remote(
    root: &Path,
    branch: Option<&str>,
    requested: Option<&str>,
) -> Result<String, SyncFailure> {
    let remotes = configured_remotes(root);
    if let Some(requested) = requested.map(str::trim).filter(|r| !r.is_empty()) {
        return validate_remote(requested, &remotes).map_err(SyncFailure::bad);
    }
    if let Some(branch) = branch {
        let key = format!("branch.{branch}.remote");
        if let Ok(configured) = git_stdout(root, &["config", "--get", &key]) {
            if remotes.contains(&configured) {
                return Ok(configured);
            }
        }
    }
    if remotes.iter().any(|name| name == "origin") {
        return Ok("origin".to_string());
    }
    match remotes.as_slice() {
        [only] => Ok(only.clone()),
        [] => Err(SyncFailure::bad("this repository has no remote configured")),
        _ => Err(SyncFailure::bad(
            "several remotes are configured; name one to use",
        )),
    }
}

/// `(remote, branch)` of the branch's upstream, when it has one.
fn upstream_of(root: &Path, branch: &str) -> Option<(String, String)> {
    let remote = git_stdout(
        root,
        &["config", "--get", &format!("branch.{branch}.remote")],
    )
    .ok()?;
    let merge = git_stdout(
        root,
        &["config", "--get", &format!("branch.{branch}.merge")],
    )
    .ok()?;
    let name = merge.strip_prefix("refs/heads/")?.to_string();
    (!remote.is_empty() && !name.is_empty()).then_some((remote, name))
}

fn has_unmerged(root: &Path) -> bool {
    git_stdout(root, &["diff", "--name-only", "--diff-filter=U"])
        .map(|names| !names.is_empty())
        .unwrap_or(false)
}

fn respond<T: serde::Serialize>(result: Result<T, SyncFailure>) -> Response {
    match result {
        Ok(response) => json_response(&response),
        Err(failure) => err_response(failure.status, &failure.message),
    }
}

async fn session_root(state: &DaemonState, session: &str) -> Result<PathBuf, SyncFailure> {
    let Some(dir) = session_dir(state, session).await else {
        return Err(SyncFailure {
            status: axum::http::StatusCode::NOT_FOUND,
            message: "no such session".to_string(),
        });
    };
    tokio::task::spawn_blocking(move || repo_root_of(&dir))
        .await
        .map_err(|_| SyncFailure::bad("git failed"))?
        .map_err(SyncFailure::bad)
}

async fn refuse_while_busy(
    state: &DaemonState,
    session: &str,
    action: &str,
) -> Result<(), SyncFailure> {
    match session_git_context(state, session).await {
        Some(context) if context.busy => Err(SyncFailure {
            status: axum::http::StatusCode::CONFLICT,
            message: format!("the session is mid-turn; wait for it to finish before {action}"),
        }),
        _ => Ok(()),
    }
}

/// `POST /api/git/fetch` `{session, remote?}` — fetch (and prune) one remote, or all of them.
pub(crate) async fn git_fetch(
    State(state): State<Arc<DaemonState>>,
    Json(request): Json<GitSyncRequest>,
) -> Response {
    respond(fetch_impl(&state, request).await)
}

async fn fetch_impl(
    state: &DaemonState,
    request: GitSyncRequest,
) -> Result<GitSyncResponse, SyncFailure> {
    let root = session_root(state, &request.session).await?;
    let named = request
        .remote
        .as_deref()
        .map(str::trim)
        .filter(|r| !r.is_empty());
    let remote = match named {
        Some(_) => Some(resolve_remote(&root, None, named)?),
        None => None,
    };
    let mut args = vec!["fetch".to_string(), "--prune".to_string()];
    match &remote {
        Some(remote) => args.push(remote.clone()),
        None => args.push("--all".to_string()),
    }
    let output = run_network_git(&root, &args)
        .await
        .map_err(SyncFailure::bad)?;
    Ok(GitSyncResponse {
        ok: true,
        remote,
        branch: None,
        output,
    })
}

/// `POST /api/git/pull` `{session, remote?, strategy?}`.
pub(crate) async fn git_pull(
    State(state): State<Arc<DaemonState>>,
    Json(request): Json<GitSyncRequest>,
) -> Response {
    respond(pull_impl(&state, request).await)
}

async fn pull_impl(
    state: &DaemonState,
    request: GitSyncRequest,
) -> Result<GitSyncResponse, SyncFailure> {
    let root = session_root(state, &request.session).await?;
    refuse_while_busy(state, &request.session, "pulling").await?;
    let strategy = match request.strategy.as_deref().map(str::trim) {
        None | Some("") | Some("ff") => "--ff-only",
        Some("merge") => "--no-rebase",
        Some("rebase") => "--rebase",
        Some(other) => {
            return Err(SyncFailure::bad(format!(
                "unknown pull strategy '{other}' — use ff, merge or rebase"
            )))
        }
    };
    let branch = current_branch(&root)?;
    let upstream = upstream_of(&root, &branch);
    let mut args = vec!["pull".to_string(), strategy.to_string()];
    let remote = match (&request.remote, &upstream) {
        (None, Some(_)) => None,
        _ => Some(resolve_remote(
            &root,
            Some(&branch),
            request.remote.as_deref(),
        )?),
    };
    if let Some(remote) = &remote {
        args.push(remote.clone());
        args.push(
            upstream
                .as_ref()
                .map(|(_, name)| name.clone())
                .unwrap_or_else(|| branch.clone()),
        );
    }
    match run_network_git(&root, &args).await {
        Ok(output) => Ok(GitSyncResponse {
            ok: true,
            remote,
            branch: Some(branch),
            output,
        }),
        Err(output) => {
            let unmerged = {
                let root = root.clone();
                tokio::task::spawn_blocking(move || has_unmerged(&root))
                    .await
                    .unwrap_or(false)
            };
            if unmerged {
                Err(SyncFailure {
                    status: axum::http::StatusCode::CONFLICT,
                    message:
                        "the pull stopped on merge conflicts; resolve them from the conflicts list"
                            .to_string(),
                })
            } else {
                Err(SyncFailure::bad(output))
            }
        }
    }
}

/// `POST /api/git/push` `{session, remote?}` — never forced. A branch with no upstream is pushed
/// with `--set-upstream` so the next push and the dock's ahead/behind counts know where it went.
pub(crate) async fn git_push(
    State(state): State<Arc<DaemonState>>,
    Json(request): Json<GitSyncRequest>,
) -> Response {
    respond(push_impl(&state, request).await)
}

async fn push_impl(
    state: &DaemonState,
    request: GitSyncRequest,
) -> Result<GitSyncResponse, SyncFailure> {
    let root = session_root(state, &request.session).await?;
    let branch = current_branch(&root)?;
    let upstream = upstream_of(&root, &branch);
    let explicit = request
        .remote
        .as_deref()
        .map(str::trim)
        .filter(|r| !r.is_empty());
    let (remote, args) = match (&upstream, explicit) {
        (Some((remote, name)), None) => (
            remote.clone(),
            vec![
                "push".to_string(),
                remote.clone(),
                format!("HEAD:refs/heads/{name}"),
            ],
        ),
        (Some((_, name)), Some(_)) => {
            let remote = resolve_remote(&root, Some(&branch), explicit)?;
            (
                remote.clone(),
                vec![
                    "push".to_string(),
                    remote,
                    format!("HEAD:refs/heads/{name}"),
                ],
            )
        }
        (None, _) => {
            let remote = resolve_remote(&root, Some(&branch), explicit)?;
            (
                remote.clone(),
                vec![
                    "push".to_string(),
                    "--set-upstream".to_string(),
                    remote,
                    format!("HEAD:refs/heads/{branch}"),
                ],
            )
        }
    };
    let output = run_network_git(&root, &args)
        .await
        .map_err(SyncFailure::bad)?;
    Ok(GitSyncResponse {
        ok: true,
        remote: Some(remote),
        branch: Some(branch),
        output,
    })
}

/// `POST /api/git/discard` `{session, paths[]}` — throw away working-tree changes: tracked files
/// go back to what the index holds, untracked files are deleted. Staged changes are untouched
/// (unstage them first); a conflicted path is refused by git itself.
pub(crate) async fn git_discard(
    State(state): State<Arc<DaemonState>>,
    Json(request): Json<GitDiscardRequest>,
) -> Response {
    respond(discard_impl(&state, request).await)
}

async fn discard_impl(
    state: &DaemonState,
    request: GitDiscardRequest,
) -> Result<GitDiscardResponse, SyncFailure> {
    let root = session_root(state, &request.session).await?;
    let paths = repo_relative_all(&request.paths).map_err(SyncFailure::bad)?;
    refuse_while_busy(state, &request.session, "discarding changes").await?;
    tokio::task::spawn_blocking(move || discard_paths(&root, &paths))
        .await
        .map_err(|_| SyncFailure::bad("git discard failed"))?
        .map(|discarded| GitDiscardResponse {
            ok: true,
            discarded,
        })
        .map_err(SyncFailure::bad)
}

pub(super) fn discard_paths(root: &Path, paths: &[String]) -> Result<usize, String> {
    let (untracked, tracked): (Vec<&String>, Vec<&String>) =
        paths.iter().partition(|path| is_untracked(root, path));
    if !tracked.is_empty() {
        let mut args = vec!["restore", "--worktree", "--"];
        args.extend(tracked.iter().map(|path| path.as_str()));
        git_raw(root, &args)?;
    }
    if !untracked.is_empty() {
        let mut args = vec!["clean", "--force", "--"];
        args.extend(untracked.iter().map(|path| path.as_str()));
        git_raw(root, &args)?;
    }
    Ok(paths.len())
}

#[cfg(test)]
mod tests;
