//! `forge serve`'s git review dock — the REST surface behind the Machined "review" screens.
//!
//! Routes (all token-scoped under `{base}`, registered in [`crate::serve::daemon_router`]):
//! - `GET  /api/git/status?session=<id>`                    staged / unstaged / untracked rows
//! - `GET  /api/git/branches?session=<id>&q=<query>`        local / remote refs + worktree owners
//! - `GET  /api/git/diff?session=<id>&path=<p>&staged=<b>`  one file's unified-diff hunks
//! - `POST /api/git/stage`   `{session, paths[]}`
//! - `POST /api/git/unstage` `{session, paths[]}`
//! - `POST /api/git/commit`  `{session, message}` → the new commit sha
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

use crate::serve::{
    err_response, git_stdout, json_response, worktree_repo_and_branch, DaemonState,
};

mod branches;
pub(crate) use branches::{git_branches, git_create_branch, git_switch_branch};

/// Hunk lines returned per file. A review dock needs the whole change, but an unbounded diff of a
/// generated file would push megabytes at a phone — the excess is reported as `skipped_lines`.
const MAX_DIFF_LINES: usize = 4_000;

/// Rows per status bucket. `git status` on a repo with a huge untracked tree must not turn into an
/// unbounded response.
const MAX_STATUS_ROWS: usize = 2_000;

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
    /// Rows dropped by [`MAX_STATUS_ROWS`], summed across the three buckets.
    truncated: usize,
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
    skipped_lines: usize,
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

/// The directory a session's git commands run in: its worktree when it has one (that is where the
/// agent is actually editing), otherwise its cwd. `None` = no such session.
async fn session_dir(state: &DaemonState, session: &str) -> Option<String> {
    let handle = state.registry.get(session).await?;
    Some(
        handle
            .worktree
            .clone()
            .unwrap_or_else(|| handle.cwd.clone()),
    )
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
    let mut full = vec!["-C", root.to_str().unwrap_or(".")];
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

// ---------------------------------------------------------------------------
// Status
// ---------------------------------------------------------------------------

/// Parse `git status --porcelain=v1 -z`: `XY <path>\0`, with an extra NUL-separated ORIGIN path
/// following a rename/copy record. NUL framing (not the default line framing) so paths containing
/// spaces, quotes, or newlines survive verbatim.
fn parse_status_z(raw: &str) -> Vec<(char, char, String, Option<String>)> {
    let mut out = Vec::new();
    let mut fields = raw.split('\0');
    while let Some(record) = fields.next() {
        if record.len() < 4 {
            continue;
        }
        let mut chars = record.chars();
        let (Some(x), Some(y)) = (chars.next(), chars.next()) else {
            continue;
        };
        let path = record[3..].to_string();
        let orig = if x == 'R' || x == 'C' || y == 'R' || y == 'C' {
            fields.next().map(str::to_string)
        } else {
            None
        };
        out.push((x, y, path, orig));
    }
    out
}

/// Parse `git diff --numstat -z` into `path → (adds, dels, binary)`.
///
/// Normal record: `adds\tdels\tpath\0`. Rename record: `adds\tdels\t\0old\0new\0` — the path field
/// is empty and the two names follow, so the fields must be walked in order rather than split
/// independently. Binary files report `-` for both counts.
fn parse_numstat_z(raw: &str) -> std::collections::HashMap<String, (usize, usize, bool)> {
    let mut out = std::collections::HashMap::new();
    let mut fields = raw.split('\0');
    while let Some(record) = fields.next() {
        let record = record.trim_start_matches('\n');
        if record.trim().is_empty() {
            continue;
        }
        let mut parts = record.splitn(3, '\t');
        let (Some(adds), Some(dels), Some(rest)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let binary = adds == "-" || dels == "-";
        let adds = adds.parse::<usize>().unwrap_or(0);
        let dels = dels.parse::<usize>().unwrap_or(0);
        let path = if rest.is_empty() {
            // Rename/copy: `\0old\0new\0` — the destination name is what status reports.
            let _old = fields.next();
            match fields.next() {
                Some(new) => new.to_string(),
                None => continue,
            }
        } else {
            rest.to_string()
        };
        out.insert(path, (adds, dels, binary));
    }
    out
}

fn status_rows(
    entries: &[(char, char, String, Option<String>)],
    numstat: &std::collections::HashMap<String, (usize, usize, bool)>,
    pick: impl Fn(char, char) -> Option<char>,
    truncated: &mut usize,
) -> Vec<GitFileRow> {
    let mut rows = Vec::new();
    for (x, y, path, orig) in entries {
        let Some(letter) = pick(*x, *y) else { continue };
        if rows.len() >= MAX_STATUS_ROWS {
            *truncated += 1;
            continue;
        }
        let (adds, dels, binary) = numstat.get(path).copied().unwrap_or((0, 0, false));
        rows.push(GitFileRow {
            path: path.clone(),
            status: letter.to_string(),
            orig_path: orig.clone(),
            adds,
            dels,
            binary,
        });
    }
    rows
}

/// `GET /api/git/status?session=<id>` — the review dock's file list, bucketed exactly as git sees
/// it (a file edited after being staged legitimately appears in BOTH `staged` and `unstaged`).
pub(crate) async fn git_status(
    State(state): State<Arc<DaemonState>>,
    Query(params): Query<GitSessionQuery>,
) -> Response {
    let Some(dir) = session_dir(&state, &params.session).await else {
        return no_such_session();
    };
    let base_repo = worktree_repo_and_branch(&dir).map(|(repo, _)| repo);
    let result = tokio::task::spawn_blocking(move || -> Result<GitStatusResponse, String> {
        let root = repo_root_of(&dir)?;
        let base_branch = base_repo.and_then(|repo| {
            git_stdout(&repo, &["symbolic-ref", "--quiet", "--short", "HEAD"]).ok()
        });
        let status = String::from_utf8_lossy(&git_raw(
            &root,
            &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        )?)
        .into_owned();
        let entries = parse_status_z(&status);
        let unstaged_stat = parse_numstat_z(&String::from_utf8_lossy(&git_raw(
            &root,
            &["diff", "--numstat", "-z"],
        )?));
        let staged_stat = parse_numstat_z(&String::from_utf8_lossy(&git_raw(
            &root,
            &["diff", "--cached", "--numstat", "-z"],
        )?));
        let branch = git_stdout(&root, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_default();
        let mut truncated = 0usize;
        let staged = status_rows(
            &entries,
            &staged_stat,
            |x, _| (x != ' ' && x != '?').then_some(x),
            &mut truncated,
        );
        let unstaged = status_rows(
            &entries,
            &unstaged_stat,
            |_, y| (y != ' ' && y != '?').then_some(y),
            &mut truncated,
        );
        let untracked = status_rows(
            &entries,
            &std::collections::HashMap::new(),
            |x, y| (x == '?' && y == '?').then_some('?'),
            &mut truncated,
        );
        Ok(GitStatusResponse {
            root: root.to_string_lossy().into_owned(),
            branch,
            base_branch,
            staged,
            unstaged,
            untracked,
            truncated,
        })
    })
    .await;
    match result {
        Ok(Ok(response)) => json_response(&response),
        Ok(Err(message)) => err_response(axum::http::StatusCode::BAD_REQUEST, &message),
        Err(_) => err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "git status failed",
        ),
    }
}

// ---------------------------------------------------------------------------
// Unified-diff parsing
// ---------------------------------------------------------------------------

/// Path out of a `diff --git a/<x> b/<y>` header. Git quotes non-ASCII/space paths in this header
/// unless `core.quotePath=false` is set (the callers do), so a plain split is faithful.
fn header_paths(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("diff --git ")?;
    let (a, b) = rest.split_once(" b/")?;
    Some((a.strip_prefix("a/").unwrap_or(a).to_string(), b.to_string()))
}

/// Parse a multi-file unified diff into [`GitDiffFile`] rows, capping hunk lines per file.
fn parse_unified_diff(raw: &str) -> Vec<GitDiffFile> {
    let mut files: Vec<GitDiffFile> = Vec::new();
    let mut current: Option<GitDiffFile> = None;
    for line in raw.split('\n') {
        if line.starts_with("diff --git ") {
            if let Some(file) = current.take() {
                files.push(file);
            }
            let (_, path) = header_paths(line).unwrap_or_default();
            current = Some(GitDiffFile {
                path,
                kind: "modified".to_string(),
                orig_path: None,
                binary: false,
                adds: 0,
                dels: 0,
                hunks: Vec::new(),
                skipped_lines: 0,
            });
            continue;
        }
        let Some(file) = current.as_mut() else {
            continue;
        };
        if line.starts_with("new file mode") {
            file.kind = "created".to_string();
            file.orig_path = None;
            continue;
        }
        if line.starts_with("deleted file mode") {
            file.kind = "deleted".to_string();
            file.orig_path = None;
            continue;
        }
        if let Some(rename) = line.strip_prefix("rename from ") {
            file.kind = "renamed".to_string();
            file.orig_path = Some(rename.to_string());
            continue;
        }
        if line.starts_with("Binary files ") || line.starts_with("GIT binary patch") {
            file.binary = true;
            continue;
        }
        if line.starts_with("@@") {
            file.hunks.push(GitDiffHunk {
                header: line.to_string(),
                lines: Vec::new(),
            });
            continue;
        }
        // Everything before the first `@@` (index/mode/---/+++ lines) is metadata, not content.
        if file.hunks.is_empty() {
            continue;
        }
        match line.as_bytes().first() {
            Some(b'+') => file.adds += 1,
            Some(b'-') => file.dels += 1,
            Some(b' ') => {}
            // `\ No newline at end of file` and the blank line git emits after a diff.
            _ => continue,
        }
        let used: usize = file.hunks.iter().map(|hunk| hunk.lines.len()).sum();
        if used >= MAX_DIFF_LINES {
            file.skipped_lines += 1;
            continue;
        }
        if let Some(hunk) = file.hunks.last_mut() {
            hunk.lines.push(line.to_string());
        }
    }
    if let Some(file) = current.take() {
        files.push(file);
    }
    files
}

/// Render an untracked file as an all-additions diff. `git diff` cannot produce one without
/// `--no-index` (which is not portable across the daemon's target platforms), so the content is
/// read directly — bounded by [`MAX_UNTRACKED_BYTES`] and by [`MAX_DIFF_LINES`].
fn untracked_diff(root: &Path, rel: &str) -> Result<GitDiffFile, String> {
    let full = root.join(rel);
    let meta = std::fs::metadata(&full).map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("not a regular file".to_string());
    }
    let mut file = GitDiffFile {
        path: rel.to_string(),
        kind: "created".to_string(),
        orig_path: None,
        binary: false,
        adds: 0,
        dels: 0,
        hunks: Vec::new(),
        skipped_lines: 0,
    };
    if meta.len() > MAX_UNTRACKED_BYTES {
        file.binary = true;
        return Ok(file);
    }
    let Ok(text) = std::fs::read_to_string(&full) else {
        file.binary = true;
        return Ok(file);
    };
    let mut lines: Vec<String> = Vec::new();
    for line in text.lines() {
        file.adds += 1;
        if lines.len() >= MAX_DIFF_LINES {
            file.skipped_lines += 1;
            continue;
        }
        lines.push(format!("+{line}"));
    }
    file.hunks.push(GitDiffHunk {
        header: format!("@@ -0,0 +1,{} @@", file.adds),
        lines,
    });
    Ok(file)
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
    let result = tokio::task::spawn_blocking(move || -> Result<GitDiffResponse, String> {
        let root = repo_root_of(&dir)?;
        let mut args = vec!["-c", "core.quotePath=false", "diff"];
        if staged {
            args.push("--cached");
        }
        args.extend_from_slice(&["--find-renames", "--", rel.as_str()]);
        let raw = String::from_utf8_lossy(&git_raw(&root, &args)?).into_owned();
        let mut files = parse_unified_diff(&raw);
        if files.is_empty() && !staged {
            // No tracked diff — the file is either untracked or unchanged. `untracked_diff`
            // distinguishes them: a missing file yields an empty `files`, not a fabricated hunk.
            if let Ok(file) = untracked_diff(&root, &rel) {
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
mod tests {
    use super::branches::{is_forge_managed_worktree, parse_branch_rows, parse_worktree_branches};
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
}
