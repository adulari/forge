//! `GET /api/projects/changes?cwd=<dir>` — every live session on a project, with the files each
//! one has changed, in one call (the IDE mode's change queue; saves an N+1 of status/diff reads and
//! the client-side guesswork about who edited what).
//!
//! - A **worktree** session's files are its worktree against the fork point — the same machinery as
//!   `GET /api/sessions/{id}/diff` ([`worktree_diff`]).
//! - **Shared-tree** sessions all edit one checkout, so the tree's `git status` is attributed to
//!   them from the store's `write_file`/`edit_file` records made since the last commit. A file
//!   several sessions edited is listed under each of them. Dirty files nobody recorded an edit for
//!   (the user's own, a shell command's, a session that has since ended) go under a synthetic
//!   `unattributed` group. Shell-made changes (`sed -i`, a formatter) are not tool-call edits, so
//!   they are never attributed to a session — that limit is inherent to the record.
//!
//! `cwd` only selects which sessions to report: git runs in the sessions' own directories, never
//! in the one the client named. `content_hash` identifies the file's current bytes, stable across
//! daemon restarts, so a client can drop a "reviewed" mark exactly when the content moves.

use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};

use super::status::{parse_numstat_z, parse_status_z};
use super::*;

/// Files listed per session; the rest are counted in `truncated`.
const MAX_FILES_PER_SESSION: usize = 500;
/// Files larger than this are hashed by size and mtime instead of content.
const MAX_HASHED_BYTES: u64 = 16 * 1024 * 1024;
pub(crate) const UNATTRIBUTED_ID: &str = "unattributed";

#[derive(serde::Deserialize)]
pub(crate) struct ProjectChangesQuery {
    #[serde(default)]
    cwd: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(crate) struct ChangedFile {
    path: String,
    /// "created" | "modified" | "deleted" | "renamed".
    kind: String,
    adds: usize,
    dels: usize,
    content_hash: String,
}

#[derive(Debug, serde::Serialize)]
pub(crate) struct SessionChanges {
    /// A session id, or [`UNATTRIBUTED_ID`] for the synthetic group.
    session_id: String,
    title: String,
    branch: String,
    worktree: Option<String>,
    waiting: bool,
    busy: bool,
    files: Vec<ChangedFile>,
    /// Files past [`MAX_FILES_PER_SESSION`] that are not listed.
    truncated: usize,
}

#[derive(Debug, serde::Serialize)]
pub(crate) struct ProjectChangesResponse {
    /// The canonical directory the request resolved to.
    cwd: String,
    sessions: Vec<SessionChanges>,
}

/// A live session as the route sees it, from the registry or the terminals' presence rows.
#[derive(Debug, Clone)]
pub(super) struct LiveSession {
    pub(super) id: String,
    pub(super) title: String,
    pub(super) cwd: String,
    pub(super) worktree: Option<String>,
    pub(super) busy: bool,
    pub(super) waiting: bool,
}

async fn live_sessions(state: &DaemonState) -> Vec<LiveSession> {
    let mut rows = Vec::new();
    let mut driven = BTreeSet::new();
    for handle in state.registry.all().await {
        driven.insert(handle.session_id.clone());
        let snap = handle.snapshot_rx.borrow().snapshot.clone();
        rows.push(LiveSession {
            id: handle.session_id.clone(),
            title: handle.title(),
            cwd: handle.cwd.clone(),
            worktree: handle.worktree.clone(),
            busy: snap.busy,
            waiting: snap.permission_prompt.is_some() || snap.question.is_some(),
        });
    }
    let store = state.store.clone();
    let local = tokio::task::spawn_blocking(move || store.local_live_sessions())
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default();
    for row in local {
        if driven.contains(&row.id) {
            continue;
        }
        rows.push(LiveSession {
            id: row.id,
            title: row.title.unwrap_or_default(),
            cwd: row.cwd,
            worktree: row.worktree_path,
            busy: row.busy,
            waiting: false,
        });
    }
    rows
}

/// `GET /api/projects/changes?cwd=<dir>`.
pub(crate) async fn project_changes(
    State(state): State<Arc<DaemonState>>,
    Query(query): Query<ProjectChangesQuery>,
) -> Response {
    use axum::http::StatusCode;
    if query.cwd.trim().is_empty() {
        return err_response(StatusCode::BAD_REQUEST, "cwd is required");
    }
    let sessions = live_sessions(&state).await;
    let store = state.store.clone();
    let cwd = query.cwd;
    let result =
        tokio::task::spawn_blocking(move || collect_project_changes(&store, &cwd, sessions)).await;
    match result {
        Ok(Ok(response)) => json_response(&response),
        Ok(Err(message)) => err_response(StatusCode::BAD_REQUEST, &message),
        Err(_) => err_response(StatusCode::INTERNAL_SERVER_ERROR, "reading changes failed"),
    }
}

fn canonical(path: &str) -> Option<PathBuf> {
    std::fs::canonicalize(path).ok()
}

/// The checkout a session belongs to: a worktree session's base repository, else the repository
/// enclosing its directory (the directory itself outside git).
fn project_root_of(session: &LiveSession) -> Option<PathBuf> {
    if let Some(worktree) = &session.worktree {
        if let Some((repo, _)) = worktree_repo_and_branch(worktree) {
            return std::fs::canonicalize(repo).ok();
        }
    }
    canonical(&repo_root_or_dir(&session.cwd).to_string_lossy())
}

pub(super) fn collect_project_changes(
    store: &forge_store::Store,
    cwd: &str,
    sessions: Vec<LiveSession>,
) -> Result<ProjectChangesResponse, String> {
    let wanted = canonical(cwd).ok_or_else(|| "cwd is not a directory on this host".to_string())?;
    let on_project: Vec<(LiveSession, PathBuf)> = sessions
        .into_iter()
        .filter_map(|session| {
            let root = project_root_of(&session)?;
            wanted.starts_with(&root).then_some((session, root))
        })
        .collect();

    let mut out = Vec::new();
    let mut shared: BTreeMap<PathBuf, Vec<LiveSession>> = BTreeMap::new();
    for (session, root) in on_project {
        match session.worktree.clone() {
            Some(worktree) => out.push(worktree_session_changes(session, &worktree)),
            None => shared.entry(root).or_default().push(session),
        }
    }
    for (root, members) in shared {
        out.extend(shared_tree_changes(store, &root, members));
    }
    Ok(ProjectChangesResponse {
        cwd: wanted.to_string_lossy().into_owned(),
        sessions: out,
    })
}

fn worktree_session_changes(session: LiveSession, worktree: &str) -> SessionChanges {
    let (branch, files) = match worktree_diff(worktree) {
        Ok(diff) => {
            let files = diff
                .files
                .iter()
                .map(|file| ChangedFile {
                    path: file.path.clone(),
                    kind: file.kind.clone(),
                    adds: file.adds,
                    dels: file.dels,
                    content_hash: content_hash(&Path::new(worktree).join(&file.path)),
                })
                .collect::<Vec<_>>();
            (diff.branch, files)
        }
        Err(_) => (String::new(), Vec::new()),
    };
    changes_row(session, Some(worktree.to_string()), branch, files)
}

fn changes_row(
    session: LiveSession,
    worktree: Option<String>,
    branch: String,
    mut files: Vec<ChangedFile>,
) -> SessionChanges {
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let truncated = files.len().saturating_sub(MAX_FILES_PER_SESSION);
    files.truncate(MAX_FILES_PER_SESSION);
    SessionChanges {
        session_id: session.id,
        title: session.title,
        branch,
        worktree,
        waiting: session.waiting,
        busy: session.busy,
        files,
        truncated,
    }
}

/// The checkout's dirty files, attributed to `members` (all of which edit the same tree).
fn shared_tree_changes(
    store: &forge_store::Store,
    root: &Path,
    members: Vec<LiveSession>,
) -> Vec<SessionChanges> {
    let branch = git_stdout(root, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_default();
    let dirty = tree_files(root);
    let since = git_stdout(root, &["log", "-1", "--format=%ct"])
        .ok()
        .and_then(|secs| secs.parse::<i64>().ok())
        .unwrap_or(0);
    let ids: Vec<String> = members.iter().map(|s| s.id.clone()).collect();
    let editors = editors_by_path(store, root, &ids, since);

    let mut per_session: BTreeMap<String, Vec<ChangedFile>> = BTreeMap::new();
    let mut unattributed = Vec::new();
    for file in dirty {
        match editors.get(&file.path) {
            Some(sessions) => {
                for id in sessions {
                    per_session
                        .entry(id.clone())
                        .or_default()
                        .push(file.clone());
                }
            }
            None => unattributed.push(file),
        }
    }
    let mut rows: Vec<SessionChanges> = members
        .into_iter()
        .map(|session| {
            let files = per_session.remove(&session.id).unwrap_or_default();
            changes_row(session, None, branch.clone(), files)
        })
        .collect();
    if !unattributed.is_empty() {
        let synthetic = LiveSession {
            id: UNATTRIBUTED_ID.to_string(),
            title: "Unattributed changes".to_string(),
            cwd: root.to_string_lossy().into_owned(),
            worktree: None,
            busy: false,
            waiting: false,
        };
        rows.push(changes_row(synthetic, None, branch, unattributed));
    }
    rows
}

/// `relative path → sessions that edited it since `since``.
fn editors_by_path(
    store: &forge_store::Store,
    root: &Path,
    session_ids: &[String],
    since: i64,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut map: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let events = store
        .session_edit_events(session_ids, since)
        .unwrap_or_default();
    for event in events {
        let resolved = crate::blame::resolve_edit_path(&event.base_dir, &event.path);
        let Ok(relative) = normalize(&resolved)
            .strip_prefix(root)
            .map(Path::to_path_buf)
        else {
            continue;
        };
        map.entry(relative.to_string_lossy().replace('\\', "/"))
            .or_default()
            .insert(event.session_id);
    }
    map
}

/// Canonicalize, tolerating a file that no longer exists (a deleted file still has an edit record).
fn normalize(path: &Path) -> PathBuf {
    if let Ok(full) = path.canonicalize() {
        return full;
    }
    match (path.parent().map(Path::canonicalize), path.file_name()) {
        (Some(Ok(parent)), Some(name)) => parent.join(name),
        _ => path.to_path_buf(),
    }
}

/// Every changed file of the checkout against HEAD (staged and unstaged merged), plus untracked.
fn tree_files(root: &Path) -> Vec<ChangedFile> {
    let status = git_raw(
        root,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )
    .map(|raw| String::from_utf8_lossy(&raw).into_owned())
    .unwrap_or_default();
    // No HEAD yet (a fresh repository) makes `diff HEAD` fail; counts are then simply zero.
    let numstat = git_raw(root, &["diff", "HEAD", "--numstat", "-z"])
        .map(|raw| parse_numstat_z(&String::from_utf8_lossy(&raw)))
        .unwrap_or_default();
    let mut budget: u64 = 16 * 1024 * 1024;
    let mut files = Vec::new();
    for (x, y, path, _) in parse_status_z(&status) {
        // `.forge/` is Forge's own scratch (worktree checkouts, session notes), never a change
        // the user or a session made to their project.
        if x == '!' || path.starts_with(".forge/") {
            continue;
        }
        let kind = match (x, y) {
            ('?', '?') | ('A', _) | (_, 'A') => "created",
            ('D', _) | (_, 'D') => "deleted",
            ('R', _) | (_, 'R') => "renamed",
            _ => "modified",
        };
        let (adds, dels) = if kind == "created" && x == '?' {
            super::diff::untracked_counts(root, &path, &mut budget)
                .map_or((0, 0), |(adds, dels, _)| (adds, dels))
        } else {
            numstat
                .get(&path)
                .map_or((0, 0), |(adds, dels, _)| (*adds, *dels))
        };
        files.push(ChangedFile {
            content_hash: content_hash(&root.join(&path)),
            path,
            kind: kind.to_string(),
            adds,
            dels,
        });
    }
    files
}

/// A short, stable digest of what is at `path` right now. `deleted` when nothing is.
pub(super) fn content_hash(path: &Path) -> String {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return "deleted".to_string();
    };
    let mut hasher = Sha256::new();
    if meta.file_type().is_symlink() {
        hasher.update(b"link:");
        if let Ok(target) = std::fs::read_link(path) {
            hasher.update(target.to_string_lossy().as_bytes());
        }
    } else if meta.is_file() && meta.len() <= MAX_HASHED_BYTES {
        match std::fs::read(path) {
            Ok(bytes) => hasher.update(&bytes),
            Err(_) => return "unreadable".to_string(),
        }
    } else {
        let modified = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_nanos());
        hasher.update(format!("meta:{}:{modified}", meta.len()).as_bytes());
    }
    hex::encode(&hasher.finalize()[..8])
}

#[cfg(test)]
mod tests;
