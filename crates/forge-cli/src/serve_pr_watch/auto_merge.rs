//! GitHub auto-merge (`method = "auto"`) keeps merging on its own once armed, even after the
//! owner's identity pushes new commits. Forge turns it off whenever it stops vouching for the head
//! it armed: a new head (not yet reviewed), a merge-queue hold, or a fix turn or block the watch
//! is about to act on. The next clean `Decision::Merge` reviews the current head and arms again.

use std::path::Path;

use serde_json::Value;

use super::{Decision, Mode, Watch};
use crate::serve_pr::{Gh, PrSummary};

/// An `auto` merge watch arms GitHub auto-merge before checks finish, but not while a fix is
/// owed: a fix turn means new code is coming, and that head is reviewed before it may merge.
pub(super) fn arms_now(
    watch: &Watch,
    pr: &PrSummary,
    comments: &[String],
    conflicting: bool,
) -> bool {
    watch.mode == Mode::Merge
        && watch.method == "auto"
        && !watch.auto_armed
        && !conflicting
        && pr.checks.failing == 0
        && pr.review_decision != "changes_requested"
        && comments.iter().all(|id| watch.seen_comments.contains(id))
}

/// Before deciding: the PR moved past the armed head, or the merge queue holds it. False when
/// turning it off failed; the poll then stops there (with the reason) and the next one retries.
pub(super) async fn disarm_if_stale(
    gh: &Gh,
    root: &Path,
    watch: &mut Watch,
    pr: &PrSummary,
    view: &Value,
) -> bool {
    let stale = watch.armed_head.as_deref() != Some(pr.head_sha.as_str()) || watch.hold.is_some();
    match disarm(gh, root, watch, pr, view, stale).await {
        Ok(()) => true,
        Err(reason) => {
            watch.blocked_reason = Some(reason);
            false
        }
    }
}

/// After deciding: a fix turn means new code is coming and a block means the watch stops, so
/// neither may leave GitHub free to merge. When turning it off fails, the watch waits instead and
/// tries again on the next poll.
pub(super) async fn disarm_before_acting(
    gh: &Gh,
    root: &Path,
    watch: &mut Watch,
    pr: &PrSummary,
    view: &Value,
    decision: &mut Decision,
) {
    let acting = matches!(decision, Decision::Fix { .. } | Decision::Block(_));
    if let Err(reason) = disarm(gh, root, watch, pr, view, acting).await {
        *decision = Decision::Wait(Some(reason));
    }
}

async fn disarm(
    gh: &Gh,
    root: &Path,
    watch: &mut Watch,
    pr: &PrSummary,
    view: &Value,
    needed: bool,
) -> Result<(), String> {
    if !watch.auto_armed || !needed || pr.state != "open" {
        return Ok(());
    }
    // `null` means GitHub already dropped it; a missing field (an older `gh`) is unknown.
    if !matches!(view.get("autoMergeRequest"), Some(Value::Null)) {
        let number = pr.number.to_string();
        gh.run(root, &["pr", "merge", &number, "--disable-auto"], None)
            .await
            .map_err(|e| {
                format!(
                    "could not turn off GitHub auto-merge ({}); will retry",
                    e.message()
                )
            })?;
    }
    watch.auto_armed = false;
    watch.armed_head = None;
    watch.last_action = Some("auto-merge turned off".into());
    Ok(())
}
