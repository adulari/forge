//! `forge serve`'s git review dock — the REST surface behind the Machined "review" screens.
//!
//! Routes (all token-scoped under `{base}`, registered in [`crate::serve::daemon_router`]):
//! - `GET  /api/git/status?session=<id>`                    staged / unstaged / untracked / conflicted rows,
//!   plus the upstream, ahead/behind counts and any merge/rebase in progress
//! - `GET  /api/git/branches?session=<id>&q=<query>`        local / remote refs + worktree owners
//! - `GET  /api/git/diff?session=<id>&path=<p>&staged=<b>[&context=<n>][&ignore_whitespace=<b>]`
//!   one file's unified-diff hunks
//! - `POST /api/git/stage`   `{session, paths[]}`
//! - `POST /api/git/unstage` `{session, paths[]}`
//! - `POST /api/git/commit`  `{session, message}` → the new commit sha
//! - `POST /api/git/commit-message` `{session, language?}` → a model-written message for the staged diff
//! - `POST /api/git/{fetch,pull,push}` `{session, remote?}` (pull also `strategy`) → never forced
//! - `POST /api/git/discard` `{session, paths[]}` → drop working-tree changes
//! - `POST /api/git/switch`  `{session, branch}` → switch a clean shared workspace
//! - `POST /api/git/branches` `{session, name}` → create + switch a clean shared workspace
//! - `GET  /api/sessions/{id}/diff`                         a forked session's worktree vs its base
//!
//! Every route resolves its repository from the SESSION (its worktree if it has one, else its
//! cwd) and then runs git with `-C <repo root>`; a client can never name the directory. Requested
//! paths are validated to stay inside that root ([`repo_relative`]) and are always passed after a
//! `--` separator, so neither `../` escapes nor `-`-prefixed option injection reach git.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::{Json, Path as AxumPath, Query, State};
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;

use crate::serve::{
    err_response, git_stdout, json_response, worktree_repo_and_branch, DaemonState,
};

/// The review dock's routes; every one is session-addressed and runs git only in that session's
/// own repository.
pub(crate) fn routes(base: &str) -> Router<Arc<DaemonState>> {
    Router::new()
        .route(&format!("{base}/api/git/status"), get(status::git_status))
        .route(
            &format!("{base}/api/git/branches"),
            get(git_branches).post(git_create_branch),
        )
        .route(&format!("{base}/api/git/switch"), post(git_switch_branch))
        .route(&format!("{base}/api/git/diff"), get(git_diff))
        .route(&format!("{base}/api/git/stage"), post(git_stage))
        .route(&format!("{base}/api/git/unstage"), post(git_unstage))
        .route(&format!("{base}/api/git/commit"), post(git_commit))
        .route(
            &format!("{base}/api/git/commit-message"),
            post(message::git_commit_message),
        )
        .route(&format!("{base}/api/git/fetch"), post(sync::git_fetch))
        .route(&format!("{base}/api/git/pull"), post(sync::git_pull))
        .route(&format!("{base}/api/git/push"), post(sync::git_push))
        .route(&format!("{base}/api/git/discard"), post(sync::git_discard))
}

mod branches;
mod diff;
mod message;
mod status;
mod sync;
pub(crate) mod url;
pub(crate) use branches::{git_branches, git_create_branch, git_switch_branch};
use diff::{
    is_untracked, parse_unified_diff, parse_unified_diff_window, staged_rename_source,
    untracked_counts, untracked_diff,
};

/// Hunk lines returned per file. A review dock needs the whole change, but an unbounded diff of a
/// generated file would push megabytes at a phone — the excess is reported as `skipped_lines`.
const MAX_DIFF_LINES: usize = 4_000;

/// Rows per status bucket. `git status` on a repo with a huge untracked tree must not turn into an
/// unbounded response.
const MAX_STATUS_ROWS: usize = 2_000;

/// Most context lines a client may ask a diff to carry (`context=`); expanding a hunk to the
/// whole file is the point, but a request must not be able to demand an unbounded `-U`.
const MAX_CONTEXT_LINES: usize = 2_000;

/// Largest untracked file rendered as an all-additions hunk (bigger ones report size only).
const MAX_UNTRACKED_BYTES: u64 = 512 * 1024;

/// Ref rows returned after server-side filtering. This is deliberately lower than the status-row
/// cap: the branch picker virtualises locally, while a pathological ref namespace must not turn a
/// phone request into a multi-megabyte response.
const DEFAULT_BRANCH_ROWS: usize = 200;
const MAX_BRANCH_ROWS: usize = 500;

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

/// One file in `GET /api/git/status`. `status` is git's porcelain letter for the bucket the row is
/// in — `M`/`A`/`D`/`R`/`C`/`T`/`U` for tracked changes, `?` for untracked.
#[derive(serde::Serialize)]
pub(crate) struct GitFileRow {
    path: String,
    status: String,
    /// Rename/copy source, when `status` is `R`/`C`.
    orig_path: Option<String>,
    adds: usize,
    dels: usize,
    binary: bool,
}

#[derive(serde::Serialize)]
pub(crate) struct GitStatusResponse {
    /// Absolute repository root every path in this response is relative to.
    root: String,
    branch: String,
    /// Set when the session runs in a forge worktree — the branch's fork point in the base repo.
    base_branch: Option<String>,
    staged: Vec<GitFileRow>,
    unstaged: Vec<GitFileRow>,
    untracked: Vec<GitFileRow>,
    /// Paths with unresolved merge conflicts. They appear here only, never in `staged` or
    /// `unstaged`, so a client cannot offer to commit or discard them by accident.
    conflicted: Vec<GitFileRow>,
    /// Rows dropped by [`MAX_STATUS_ROWS`], summed across the buckets.
    truncated: usize,
    /// The branch's upstream (`origin/main`), when it has one.
    upstream: Option<String>,
    /// Commits the branch has that its upstream lacks / the other way round. Zero without an
    /// upstream.
    ahead: usize,
    behind: usize,
    /// `merge` / `rebase` / `cherry-pick` / `revert` while one is unfinished.
    operation: Option<String>,
    /// Whether the repository has any remote — whether push/pull/fetch can mean anything.
    has_remote: bool,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct GitBranchRow {
    /// Local names are `feature/x`; remote names retain their remote prefix (`origin/feature/x`).
    name: String,
    oid: String,
    upstream: Option<String>,
    remote: bool,
    current: bool,
    default: bool,
    /// Absolute checkout path when this local branch is owned by a worktree.
    worktree: Option<String>,
}

#[derive(serde::Serialize)]
pub(crate) struct GitBranchesResponse {
    root: String,
    current: Option<String>,
    default_branch: Option<String>,
    managed_worktree: bool,
    /// Authoritative reason create/switch is unavailable. `None` means the daemon will accept a
    /// branch action if repository state has not changed before the POST arrives.
    actions_blocked_reason: Option<String>,
    branches: Vec<GitBranchRow>,
    truncated: usize,
}

/// One `@@` hunk. Deliberately the same shape as the WS snapshot's `SnapDiffHunk` so the app can
/// render a review-dock diff and a turn diff with one component.
#[derive(serde::Serialize)]
pub(crate) struct GitDiffHunk {
    header: String,
    /// Each line keeps its gutter character (`+`/`-`/` `) as its first byte.
    lines: Vec<String>,
}

#[derive(serde::Serialize)]
pub(crate) struct GitDiffFile {
    path: String,
    /// "created" | "modified" | "deleted" | "renamed".
    kind: String,
    orig_path: Option<String>,
    binary: bool,
    adds: usize,
    dels: usize,
    hunks: Vec<GitDiffHunk>,
    /// Hunk lines NOT in `hunks`: the ones before `offset` plus the ones past the line cap.
    skipped_lines: usize,
    /// First hunk line (0-based, counted across the whole file) that `hunks` carries.
    offset: usize,
    /// Every hunk line the file has, returned or not — what a client pages through with `offset`.
    total_lines: usize,
}

#[derive(serde::Serialize)]
pub(crate) struct GitDiffResponse {
    root: String,
    staged: bool,
    files: Vec<GitDiffFile>,
}

/// `GET /api/sessions/{id}/diff` — everything a fork changed relative to where it forked.
#[derive(serde::Serialize)]
pub(crate) struct SessionDiffResponse {
    /// The fork point (`git merge-base`) the diff is taken against.
    base: String,
    branch: String,
    worktree: String,
    files: Vec<GitDiffFile>,
}

#[derive(serde::Serialize)]
pub(crate) struct GitCommitResponse {
    ok: bool,
    sha: String,
    /// `git show -s --format=%s` of the new commit — what the client can echo back.
    summary: String,
}

#[derive(serde::Deserialize)]
pub(crate) struct GitSessionQuery {
    #[serde(default)]
    session: String,
}

#[derive(serde::Deserialize)]
pub(crate) struct GitBranchesQuery {
    #[serde(default)]
    session: String,
    #[serde(default)]
    q: String,
    #[serde(default = "default_branch_rows")]
    limit: usize,
}

#[derive(serde::Deserialize)]
pub(crate) struct GitDiffQuery {
    #[serde(default)]
    session: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    staged: bool,
    /// Where a staged rename came from (`orig_path` of the status row). Without it git cannot pair
    /// the new name with the deleted old one and reports a brand-new file. Detected from the index
    /// when omitted.
    #[serde(default)]
    orig_path: Option<String>,
    /// Hunk lines to skip from the start, so a reviewer can page past the line cap.
    #[serde(default)]
    offset: usize,
    /// Hunk lines to return (default and maximum: the per-file cap).
    #[serde(default)]
    limit: Option<usize>,
    /// Context lines around each change (`git diff -U<n>`); git's default of 3 when omitted.
    #[serde(default)]
    context: Option<usize>,
    /// Ignore whitespace-only changes (`git diff -w`).
    #[serde(default)]
    ignore_whitespace: bool,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GitPathsRequest {
    session: String,
    paths: Vec<String>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GitCommitRequest {
    session: String,
    message: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GitSwitchRequest {
    session: String,
    branch: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GitCreateBranchRequest {
    session: String,
    name: String,
}

#[derive(serde::Serialize)]
pub(crate) struct GitBranchActionResponse {
    ok: bool,
    branch: String,
}

// ---------------------------------------------------------------------------
// Session → repository resolution
// ---------------------------------------------------------------------------

fn default_branch_rows() -> usize {
    DEFAULT_BRANCH_ROWS
}

/// A session persisted in the store but not loaded in the registry (idle, or the daemon restarted):
/// its recorded worktree, else its cwd, and whether that is a managed worktree.
async fn stored_session_dir(state: &DaemonState, session: &str) -> Option<(String, bool)> {
    let store = state.store.clone();
    let session = session.to_string();
    tokio::task::spawn_blocking(move || {
        let worktree = store.session_worktree(&session).ok().flatten();
        match worktree {
            Some(worktree) => Some((worktree, true)),
            None => store
                .session_cwd(&session)
                .ok()
                .flatten()
                .map(|cwd| (cwd, false)),
        }
    })
    .await
    .ok()
    .flatten()
}

/// The directory a session's git commands run in: its worktree when it has one (that is where the
/// agent is actually editing), otherwise its cwd. Falls back to the store for sessions that are
/// not currently loaded, so the dock works for any session the chat can show. `None` = no such
/// session.
pub(crate) async fn session_dir(state: &DaemonState, session: &str) -> Option<String> {
    if let Some(handle) = state.registry.get(session).await {
        return Some(
            handle
                .worktree
                .clone()
                .unwrap_or_else(|| handle.cwd.clone()),
        );
    }
    stored_session_dir(state, session).await.map(|(dir, _)| dir)
}

/// Resolve the enclosing repository root of a session's directory. This is the ONLY way a route
/// obtains a path to run git in — the request never names a directory.
fn repo_root_of(dir: &str) -> Result<PathBuf, String> {
    let dir = Path::new(dir);
    if !dir.is_dir() {
        return Err("session has no working directory on this host".to_string());
    }
    match git_stdout(dir, &["rev-parse", "--show-toplevel"]) {
        Ok(root) if !root.is_empty() => Ok(PathBuf::from(root)),
        _ => Err("session directory is not inside a git repository".to_string()),
    }
}

/// Validate a client-supplied path as repo-relative and confined to the repository root.
///
/// Rejects absolute paths, any `..` component, and leading `-` (which git would read as an option
/// even after `--` in a few subcommands). Symlink escapes are not a concern here: git itself
/// refuses to stage or diff through a symlink that leaves the work tree.
fn repo_relative(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("empty path".to_string());
    }
    if trimmed.starts_with('-') {
        return Err("path must not start with '-'".to_string());
    }
    let path = Path::new(trimmed);
    if path.is_absolute() {
        return Err("path must be relative to the repository root".to_string());
    }
    for component in path.components() {
        match component {
            std::path::Component::Normal(_) | std::path::Component::CurDir => {}
            _ => return Err("path must stay inside the repository".to_string()),
        }
    }
    Ok(trimmed.replace('\\', "/"))
}

fn repo_relative_all(raw: &[String]) -> Result<Vec<String>, String> {
    if raw.is_empty() {
        return Err("no paths given".to_string());
    }
    raw.iter().map(|path| repo_relative(path)).collect()
}

/// The 404 every session-addressed git route answers with when the id isn't running.
fn no_such_session() -> Response {
    err_response(axum::http::StatusCode::NOT_FOUND, "no such session")
}

/// Run `git -C <root> <args>` and return raw (untrimmed) stdout — the diff routes need the exact
/// bytes, which [`git_stdout`]'s trim would corrupt.
fn git_raw(root: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let mut full = vec!["--literal-pathspecs", "-C", root.to_str().unwrap_or(".")];
    full.extend_from_slice(args);
    let out = std::process::Command::new("git")
        .args(&full)
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(out.stdout)
}

/// `GET /api/git/diff?session=<id>&path=<p>&staged=<bool>` — one file's hunks. `staged=true` reads
/// the index against HEAD; `staged=false` reads the working tree against the index, and falls back
/// to an all-additions render when the path is untracked (git has no diff for it).
pub(crate) async fn git_diff(
    State(state): State<Arc<DaemonState>>,
    Query(params): Query<GitDiffQuery>,
) -> Response {
    let Some(dir) = session_dir(&state, &params.session).await else {
        return no_such_session();
    };
    let rel = match repo_relative(&params.path) {
        Ok(rel) => rel,
        Err(message) => return err_response(axum::http::StatusCode::BAD_REQUEST, &message),
    };
    let staged = params.staged;
    let orig = match params.orig_path.as_deref().filter(|p| !p.trim().is_empty()) {
        Some(orig) => match repo_relative(orig) {
            Ok(orig) => Some(orig),
            Err(message) => return err_response(axum::http::StatusCode::BAD_REQUEST, &message),
        },
        None => None,
    };
    let (offset, limit) = (params.offset, params.limit.unwrap_or(MAX_DIFF_LINES));
    let unified = params
        .context
        .map(|lines| format!("-U{}", lines.min(MAX_CONTEXT_LINES)));
    let ignore_whitespace = params.ignore_whitespace;
    let result = tokio::task::spawn_blocking(move || -> Result<GitDiffResponse, String> {
        let root = repo_root_of(&dir)?;
        // A staged rename is only recognised when git sees BOTH names in the pathspec.
        let orig = orig.or_else(|| staged.then(|| staged_rename_source(&root, &rel)).flatten());
        let mut args = vec!["-c", "core.quotePath=false", "diff"];
        if staged {
            args.push("--cached");
        }
        if let Some(unified) = unified.as_deref() {
            args.push(unified);
        }
        if ignore_whitespace {
            args.push("-w");
        }
        args.extend_from_slice(&["--find-renames", "--", rel.as_str()]);
        if let Some(orig) = orig.as_deref() {
            args.push(orig);
        }
        let raw = String::from_utf8_lossy(&git_raw(&root, &args)?).into_owned();
        let mut files = parse_unified_diff_window(&raw, offset, limit);
        if files.is_empty() && !staged && is_untracked(&root, &rel) {
            // No tracked diff — the file is untracked (rendered as all additions) or unchanged
            // (no diff at all: a tracked file identical to HEAD must not show as "created").
            if let Ok(file) = untracked_diff(&root, &rel, offset, limit) {
                files.push(file);
            }
        }
        Ok(GitDiffResponse {
            root: root.to_string_lossy().into_owned(),
            staged,
            files,
        })
    })
    .await;
    match result {
        Ok(Ok(response)) => json_response(&response),
        Ok(Err(message)) => err_response(axum::http::StatusCode::BAD_REQUEST, &message),
        Err(_) => err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "git diff failed",
        ),
    }
}

// ---------------------------------------------------------------------------
// Stage / unstage / commit
// ---------------------------------------------------------------------------

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
        let mut args: Vec<&str> = if unstage {
            // `restore --staged` (not `reset`) so an unstage never moves HEAD or touches the
            // working tree — the dock only ever changes what is in the index.
            vec!["restore", "--staged", "--"]
        } else {
            vec!["add", "--"]
        };
        args.extend(paths.iter().map(String::as_str));
        git_raw(&root, &args).map(|_| ())
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

// ---------------------------------------------------------------------------
// Cross-session diff
// ---------------------------------------------------------------------------

/// `GET /api/sessions/{id}/diff` — what a forked session's worktree changed relative to the point
/// it forked from. Same machinery `POST /api/sessions/{id}/merge` uses to decide what to apply
/// ([`worktree_repo_and_branch`] + `git merge-base HEAD <branch>`), but read-only: the diff is
/// taken IN the worktree so uncommitted edits are included without committing anything.
/// Unified new-file diffs for the untracked, non-ignored files of `dir` (capped), without touching
/// the index. `git diff --no-index` exits 1 when the files differ, so the status is not checked.
fn untracked_as_new_file_diffs(dir: &Path) -> String {
    const MAX_FILES: usize = 200;
    const MAX_BYTES: u64 = 1024 * 1024;
    let Ok(listing) = git_raw(dir, &["ls-files", "--others", "--exclude-standard", "-z"]) else {
        return String::new();
    };
    let mut out = String::new();
    for rel in String::from_utf8_lossy(&listing)
        .split('\0')
        .filter(|p| !p.is_empty())
        .take(MAX_FILES)
    {
        if std::fs::metadata(dir.join(rel)).is_ok_and(|m| !m.is_file() || m.len() > MAX_BYTES) {
            continue;
        }
        let Ok(output) = std::process::Command::new("git")
            .current_dir(dir)
            .args([
                "-c",
                "core.quotePath=false",
                "diff",
                "--no-index",
                "--",
                "/dev/null",
                rel,
            ])
            .output()
        else {
            continue;
        };
        out.push_str(&String::from_utf8_lossy(&output.stdout));
    }
    out
}

pub(crate) async fn session_diff(
    State(state): State<Arc<DaemonState>>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let Some(handle) = state.registry.get(&id).await else {
        return err_response(axum::http::StatusCode::NOT_FOUND, "no such session");
    };
    let Some(worktree) = handle.worktree.clone() else {
        return err_response(
            axum::http::StatusCode::BAD_REQUEST,
            "session has no worktree to diff",
        );
    };
    let Some((repo_root, branch)) = worktree_repo_and_branch(&worktree) else {
        return err_response(
            axum::http::StatusCode::BAD_REQUEST,
            "worktree path is not a recognised forge worktree",
        );
    };
    let result = tokio::task::spawn_blocking(move || -> Result<SessionDiffResponse, String> {
        let base = git_stdout(&repo_root, &["merge-base", "HEAD", &branch])
            .map_err(|e| format!("finding the fork point failed: {e}"))?;
        let raw = String::from_utf8_lossy(&git_raw(
            Path::new(&worktree),
            &[
                "-c",
                "core.quotePath=false",
                "diff",
                "--find-renames",
                &base,
                "--",
            ],
        )?)
        .into_owned();
        // `git diff` never lists untracked files, but the merge brings them in: add them as
        // new-file diffs so the pre-merge view shows everything that will land.
        let raw = raw + &untracked_as_new_file_diffs(Path::new(&worktree));
        Ok(SessionDiffResponse {
            base,
            branch,
            worktree,
            files: parse_unified_diff(&raw),
        })
    })
    .await;
    match result {
        Ok(Ok(response)) => json_response(&response),
        Ok(Err(message)) => err_response(axum::http::StatusCode::BAD_REQUEST, &message),
        Err(_) => err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "session diff failed",
        ),
    }
}

#[cfg(test)]
mod status_tests;

#[cfg(test)]
mod tests {
    use super::branches::{is_forge_managed_worktree, parse_branch_rows, parse_worktree_branches};
    use super::status::{parse_numstat_z, parse_status_z};
    use super::*;

    #[test]
    fn worktree_porcelain_maps_local_branches_to_paths() {
        let rows = parse_worktree_branches(
            "worktree /repo\nHEAD abc\nbranch refs/heads/main\n\n\
             worktree /repo/.forge/worktrees/child\nHEAD def\nbranch refs/heads/forge/subagent/child\n\n\
             worktree /repo/detached\nHEAD 123\ndetached\n",
        );
        assert_eq!(rows.get("main").map(String::as_str), Some("/repo"));
        assert_eq!(
            rows.get("forge/subagent/child").map(String::as_str),
            Some("/repo/.forge/worktrees/child")
        );
        assert_eq!(rows.len(), 2);
        assert!(is_forge_managed_worktree("/repo/.forge/worktrees/child"));
        assert!(!is_forge_managed_worktree("/repo/other-worktree"));
    }

    #[test]
    fn branch_rows_keep_remote_identity_and_worktree_occupancy() {
        let raw = concat!(
            "refs/heads/main\0aaa\0origin/main\n",
            "refs/heads/feature/x\0bbb\0\n",
            "refs/remotes/origin/HEAD\0aaa\0\n",
            "refs/remotes/origin/main\0aaa\0\n",
            "refs/remotes/origin/feature/y\0ccc\0\n",
        );
        let worktrees =
            std::collections::HashMap::from([("feature/x".to_string(), "/repo/other".to_string())]);
        let rows = parse_branch_rows(raw, Some("main"), Some("main"), &worktrees);

        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].name, "main");
        assert!(rows[0].current);
        assert!(rows[0].default);
        let occupied = rows.iter().find(|row| row.name == "feature/x").unwrap();
        assert_eq!(occupied.worktree.as_deref(), Some("/repo/other"));
        let remote = rows
            .iter()
            .find(|row| row.name == "origin/feature/y")
            .unwrap();
        assert!(remote.remote);
        assert!(!rows.iter().any(|row| row.name == "origin/HEAD"));
    }

    #[test]
    fn repo_relative_rejects_escapes_and_options() {
        assert!(repo_relative("src/lib.rs").is_ok());
        assert!(repo_relative("./src/lib.rs").is_ok());
        assert!(repo_relative("../etc/passwd").is_err());
        assert!(repo_relative("/etc/passwd").is_err());
        assert!(repo_relative("--output=x").is_err());
        assert!(repo_relative("  ").is_err());
        assert!(repo_relative("a/../../b").is_err());
    }

    #[test]
    fn status_z_parses_renames_with_their_origin() {
        let raw = "RM g.txt\0f.txt\0A  new.txt\0?? untracked.txt\0";
        let rows = parse_status_z(raw);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].0, 'R');
        assert_eq!(rows[0].2, "g.txt");
        assert_eq!(rows[0].3.as_deref(), Some("f.txt"));
        assert_eq!(rows[1].2, "new.txt");
        assert_eq!(rows[2].0, '?');
    }

    #[test]
    fn numstat_z_parses_plain_and_rename_records() {
        let plain = parse_numstat_z("2\t1\tg.txt\0");
        assert_eq!(plain.get("g.txt").copied(), Some((2, 1, false)));
        // Rename record: the path field is EMPTY and `old`/`new` follow as their own NUL fields.
        let renamed = parse_numstat_z("3\t4\t\0f.txt\0g.txt\0");
        assert_eq!(renamed.get("g.txt").copied(), Some((3, 4, false)));
        assert!(!renamed.contains_key("f.txt"));
        let binary = parse_numstat_z("-\t-\timg.png\0");
        assert_eq!(binary.get("img.png").copied(), Some((0, 0, true)));
    }

    #[test]
    fn unified_diff_splits_files_hunks_and_counts() {
        let raw = "diff --git a/a.rs b/a.rs\nindex 111..222 100644\n--- a/a.rs\n+++ b/a.rs\n@@ -1,2 +1,3 @@\n ctx\n-old\n+new\n+extra\ndiff --git a/b.png b/b.png\nnew file mode 100644\nBinary files /dev/null and b/b.png differ\n";
        let files = parse_unified_diff(raw);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "a.rs");
        assert_eq!(files[0].adds, 2);
        assert_eq!(files[0].dels, 1);
        assert_eq!(files[0].hunks.len(), 1);
        assert_eq!(files[0].hunks[0].lines.len(), 4);
        assert_eq!(files[1].path, "b.png");
        assert_eq!(files[1].kind, "created");
        assert!(files[1].binary);
    }

    fn git_in(dir: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn repo_with(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        git_in(dir.path(), &["init", "-q", "-b", "main"]);
        for (name, body) in files {
            if let Some(parent) = Path::new(name).parent() {
                std::fs::create_dir_all(dir.path().join(parent)).unwrap();
            }
            std::fs::write(dir.path().join(name), body).unwrap();
            git_in(dir.path(), &["add", name]);
        }
        git_in(dir.path(), &["commit", "-qm", "init"]);
        dir
    }

    #[test]
    fn an_unchanged_tracked_file_is_not_untracked() {
        let dir = repo_with(&[("src/b.txt", "two\n")]);
        std::fs::write(dir.path().join("new.txt"), "x\n").unwrap();
        assert!(
            !is_untracked(dir.path(), "src/b.txt"),
            "tracked + unchanged"
        );
        assert!(is_untracked(dir.path(), "new.txt"));
        assert!(!is_untracked(dir.path(), "missing.txt"));
    }

    #[test]
    fn a_staged_rename_is_paired_with_its_source() {
        let dir = repo_with(&[("README.md", "base\n")]);
        git_in(dir.path(), &["mv", "README.md", "RENAMED.md"]);
        assert_eq!(
            staged_rename_source(dir.path(), "RENAMED.md").as_deref(),
            Some("README.md")
        );
        assert_eq!(staged_rename_source(dir.path(), "other.md"), None);
        // With both names in the pathspec git reports a rename, not a brand-new file.
        let raw = git_raw(
            dir.path(),
            &[
                "diff",
                "--cached",
                "--find-renames",
                "--",
                "RENAMED.md",
                "README.md",
            ],
        )
        .unwrap();
        let files = parse_unified_diff(&String::from_utf8_lossy(&raw));
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].kind, "renamed");
        assert_eq!(files[0].orig_path.as_deref(), Some("README.md"));
        assert!(
            files[0].hunks.is_empty(),
            "a pure rename has no content diff"
        );
    }

    #[test]
    fn untracked_files_appear_as_new_file_diffs() {
        let dir = repo_with(&[("a.txt", "a\n")]);
        std::fs::write(dir.path().join("fresh.txt"), "hello\nworld\n").unwrap();
        let files = parse_unified_diff(&untracked_as_new_file_diffs(dir.path()));
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "fresh.txt");
        assert_eq!(files[0].kind, "created");
    }

    #[test]
    fn untracked_rows_carry_the_line_counts_the_diff_route_shows() {
        let dir = repo_with(&[("a.txt", "a\n")]);
        let body: String = (0..250).map(|i| format!("line {i}\n")).collect();
        std::fs::write(dir.path().join("big.txt"), &body).unwrap();
        let mut budget = 1024 * 1024;
        assert_eq!(
            untracked_counts(dir.path(), "big.txt", &mut budget),
            Some((250, 0, false))
        );
        assert_eq!(
            untracked_diff(dir.path(), "big.txt", 0, MAX_DIFF_LINES)
                .unwrap()
                .adds,
            250
        );
        let mut spent = 0;
        assert_eq!(
            untracked_counts(dir.path(), "big.txt", &mut spent),
            None,
            "budget spent"
        );
    }

    #[test]
    fn a_diff_window_reaches_the_additions_a_head_only_cap_hides() {
        // A full rewrite: every `-` line comes first, then every `+` line.
        let mut raw = String::from(
            "diff --git a/big.txt b/big.txt\n--- a/big.txt\n+++ b/big.txt\n@@ -1,10 +1,10 @@\n",
        );
        for i in 0..10 {
            raw.push_str(&format!("-old {i}\n"));
        }
        for i in 0..10 {
            raw.push_str(&format!("+new {i}\n"));
        }
        let head = parse_unified_diff_window(&raw, 0, 4);
        assert_eq!(head[0].total_lines, 20);
        assert_eq!(head[0].skipped_lines, 16);
        assert!(head[0].hunks[0].lines.iter().all(|l| l.starts_with('-')));
        let tail = parse_unified_diff_window(&raw, 10, 4);
        assert_eq!(tail[0].offset, 10);
        assert_eq!(
            tail[0].hunks[0].lines,
            ["+new 0", "+new 1", "+new 2", "+new 3"]
        );
        assert_eq!(
            (tail[0].adds, tail[0].dels),
            (10, 10),
            "counts cover the whole file"
        );
    }
}
