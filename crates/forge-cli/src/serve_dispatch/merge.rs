//! `POST /api/dispatches/{id}/merge` — merge every `succeeded` worker of a dispatch in index order,
//! committing each clean merge on the base repo's current branch before starting the next.
//!
//! A single-session merge leaves its result staged, and the next merge refuses a dirty base: its
//! conflict path `reset --hard`s the base, which is only safe from a clean HEAD. Committing after
//! every step keeps that true for each item, so a conflict discards only its own partial apply and
//! everything merged before it is already safe in a commit.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::{Path as AxumPath, State};
use axum::response::Response;
use forge_core::dispatch::item_status;
use forge_store::DispatchRow;

use super::{dispatch_lock, load, to_json, DispatchError};
use crate::serve::{
    base_dirty_tracked, conflict_json, git_stdout, json_response, merge_session_core,
    worktree_repo_and_branch, DaemonState, SessionMerge,
};

const PROMPT_IN_COMMIT: usize = 72;

#[derive(Debug, serde::Serialize)]
pub(crate) struct MergedItem {
    index: i64,
    title: String,
    commit: Option<String>,
}

#[derive(Debug, serde::Serialize)]
pub(crate) struct StoppedAt {
    index: i64,
    title: String,
    reason: String,
    conflicts: Vec<String>,
}

#[derive(Debug, Default)]
struct Report {
    merged: Vec<MergedItem>,
    stopped_at: Option<StoppedAt>,
    remaining: Vec<i64>,
    base_branch: Option<String>,
}

enum Refusal {
    Dispatch(DispatchError),
    Dirty(Vec<String>),
}

impl From<DispatchError> for Refusal {
    fn from(error: DispatchError) -> Self {
        Self::Dispatch(error)
    }
}

pub(super) async fn merge_dispatch(
    State(state): State<Arc<DaemonState>>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let lock = dispatch_lock(&id);
    let held = lock.lock().await;
    let result = merge_locked(&state, &id).await;
    drop(held);
    match result {
        Ok((report, row)) => {
            state.registry.notify_fleet();
            json_response(&serde_json::json!({
                "merged": report.merged,
                "stopped_at": report.stopped_at,
                "remaining": report.remaining,
                "base_branch": report.base_branch,
                "dispatch": to_json(&state, row).await,
            }))
        }
        Err(Refusal::Dispatch(error)) => error.response(),
        Err(Refusal::Dirty(files)) => conflict_json(serde_json::json!({
            "error": "the base repo has uncommitted changes — commit or stash them (a merge from \
                      `w` stays staged until committed), then merge again",
            "dirty_files": files,
        })),
    }
}

async fn merge_locked(state: &DaemonState, id: &str) -> Result<(Report, DispatchRow), Refusal> {
    let row = load(&state.store, id)?;
    if !row.worktree {
        return Err(DispatchError::Invalid(
            "this dispatch ran in a shared directory — there is nothing to merge".into(),
        )
        .into());
    }
    let mut ready: Vec<(i64, String, String)> = row
        .items
        .iter()
        .filter(|i| i.status == item_status::SUCCEEDED)
        .filter_map(|i| {
            let session = i.session_id.clone()?;
            Some((i.idx, i.title.clone(), session))
        })
        .collect();
    ready.sort_by_key(|(idx, ..)| *idx);
    if ready.is_empty() {
        return Err(DispatchError::Conflict(
            "no item has succeeded yet — only succeeded items with a session are merged".into(),
        )
        .into());
    }

    let mut roots = Vec::new();
    for (_, _, session) in &ready {
        if let Some((root, _)) = session_target(state, session).await {
            if !roots.contains(&root) {
                roots.push(root);
            }
        }
    }
    if roots.is_empty() {
        roots.push(PathBuf::from(&row.cwd));
    }
    let (dirty, base_branch) = {
        let roots = roots.clone();
        tokio::task::spawn_blocking(move || {
            let dirty: Vec<String> = roots.iter().flat_map(|r| base_dirty_tracked(r)).collect();
            let branch = git_stdout(&roots[0], &["rev-parse", "--abbrev-ref", "HEAD"]).ok();
            (dirty, branch)
        })
        .await
        .unwrap_or_default()
    };
    if !dirty.is_empty() {
        return Err(Refusal::Dirty(dirty));
    }

    let mut report = Report {
        base_branch,
        ..Report::default()
    };
    let mut queue = ready.into_iter();
    for (idx, title, session) in queue.by_ref() {
        let target = session_target(state, &session).await;
        let stop = |title: &str, reason: String, conflicts: Vec<String>| StoppedAt {
            index: idx,
            title: title.to_string(),
            reason,
            conflicts,
        };
        match merge_session_core(state, &session, false).await {
            SessionMerge::Clean { branch } => {
                let _ = state
                    .store
                    .set_dispatch_item(id, idx, item_status::MERGED, None, None);
                let Some((root, _)) = target else {
                    report.merged.push(MergedItem {
                        index: idx,
                        title,
                        commit: None,
                    });
                    continue;
                };
                let (subject, body) = commit_message(&row, idx, &title, &session, &branch);
                let committed = {
                    let root = root.clone();
                    tokio::task::spawn_blocking(move || commit_staged(&root, &subject, &body))
                        .await
                        .unwrap_or_else(|e| Err(format!("commit task failed: {e}")))
                };
                match committed {
                    Ok(commit) => report.merged.push(MergedItem {
                        index: idx,
                        title,
                        commit,
                    }),
                    Err(error) => {
                        report.stopped_at = Some(stop(
                            &title,
                            format!(
                                "merged but not committed: {error} — the change is staged in {}; \
                                 commit it, then merge the rest",
                                root.display()
                            ),
                            Vec::new(),
                        ));
                        break;
                    }
                }
            }
            SessionMerge::Conflicts { files, .. } => {
                report.stopped_at = Some(stop(
                    &title,
                    "merge conflicts — resolve them by hand in the worktree; the session keeps \
                     running"
                        .into(),
                    files,
                ));
                break;
            }
            SessionMerge::Dirty(files) => {
                report.stopped_at = Some(stop(
                    &title,
                    format!(
                        "the base repo has uncommitted changes: {}",
                        files.join(", ")
                    ),
                    Vec::new(),
                ));
                break;
            }
            SessionMerge::Refused(_, message) | SessionMerge::Failed(message) => {
                report.stopped_at = Some(stop(&title, message, Vec::new()));
                break;
            }
        }
    }
    report.remaining = queue.map(|(idx, ..)| idx).collect();
    Ok((report, load(&state.store, id)?))
}

async fn session_target(state: &DaemonState, session: &str) -> Option<(PathBuf, String)> {
    let handle = state.registry.get(session).await?;
    worktree_repo_and_branch(handle.worktree.as_deref()?)
}

fn short(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}

pub(super) fn commit_message(
    row: &DispatchRow,
    idx: i64,
    title: &str,
    session: &str,
    branch: &str,
) -> (String, String) {
    let request = row.prompt.lines().next().unwrap_or("").trim();
    (
        format!("Merge dispatch item {idx}: {title}"),
        format!(
            "From Forge dispatch {} — {}. Session {}, branch {branch}.",
            short(&row.id),
            forge_types::truncate_ellipsis(request, PROMPT_IN_COMMIT),
            short(session)
        ),
    )
}

/// Commit whatever the merge staged. `Ok(None)` when nothing is staged (an empty branch).
fn commit_staged(root: &Path, subject: &str, body: &str) -> Result<Option<String>, String> {
    let nothing_staged = std::process::Command::new("git")
        .args([
            "-C",
            root.to_str().unwrap_or("."),
            "diff",
            "--cached",
            "--quiet",
        ])
        .status()
        .map_err(|e| e.to_string())?
        .success();
    if nothing_staged {
        return Ok(None);
    }
    git_stdout(root, &["commit", "-q", "-m", subject, "-m", body])?;
    git_stdout(root, &["rev-parse", "HEAD"]).map(Some)
}

#[cfg(test)]
#[path = "merge_tests.rs"]
mod tests;
