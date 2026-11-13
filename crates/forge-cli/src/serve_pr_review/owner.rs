//! The owner's say over the review gate: "merge anyway" (once, for the head under review) and
//! "dismiss finding <n>". Both are recorded on the review of that head with who and when, so a
//! new head is reviewed from scratch and inherits nothing.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Json, State};
use axum::response::Response;
use serde::{Deserialize, Serialize};

use super::{ReviewState, Verdict};
use crate::serve::{err_response, json_response, DaemonState};
use crate::serve_merge_queue::ActionError;
use crate::serve_pr_watch::{registry, Mode, Phase, Registry, Watch};

/// Who did it and when.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct Owner {
    pub(crate) by: String,
    pub(crate) at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReviewAction {
    MergeAnyway,
    /// A 1-based place in the review's finding list; `None` is the first one still blocking.
    Dismiss(Option<usize>),
}

fn label(w: &Watch) -> String {
    w.pr_number
        .map_or_else(|| "this PR".to_string(), |n| format!("PR #{n}"))
}

fn who(by: &str) -> String {
    let clean: String = by.chars().filter(|c| !c.is_control()).take(64).collect();
    let clean = clean.trim();
    if clean.is_empty() { "owner" } else { clean }.to_string()
}

fn conflict<T>(text: String) -> Result<T, ActionError> {
    Err(ActionError::Conflict(text))
}

pub(crate) fn apply(
    all: &mut HashMap<String, Watch>,
    session: &str,
    pr: Option<u64>,
    action: ReviewAction,
    by: &str,
    now: i64,
) -> Result<String, ActionError> {
    let w = all
        .get_mut(session)
        .ok_or_else(|| ActionError::NotFound(format!("no PR watch for session {session}")))?;
    let name = label(w);
    // A caller that names both acts on the PR it showed; the session's watch may follow another.
    if let Some(number) = pr.filter(|n| w.pr_number != Some(*n)) {
        return conflict(format!(
            "session {session} watches {name}, not PR #{number}"
        ));
    }
    if w.mode != Mode::Merge || w.state != Phase::Watching {
        return conflict(format!("{name} is not waiting to merge"));
    }
    let head = w.head_sha.clone().unwrap_or_default();
    let Some(review) = w.review.as_mut().filter(|r| r.head_sha == head) else {
        return conflict(format!("{name} has no review of its current head yet"));
    };
    let owner = Owner {
        by: who(by),
        at: now,
    };
    let message = match action {
        ReviewAction::MergeAnyway => {
            if review.verdict == Verdict::Clean {
                return conflict(format!(
                    "the review of {name} is clean; nothing to override"
                ));
            }
            review.merge_anyway = Some(owner.clone());
            w.last_action = Some(format!("merge anyway by {}", owner.by));
            format!(
                "{name} will merge without waiting for the review of {} (by {})",
                short(&head),
                owner.by
            )
        }
        ReviewAction::Dismiss(n) => dismiss(review, &name, n, &owner)?,
    };
    // Poll again at once rather than after the usual interval.
    w.last_checked = w.last_checked.map(|_| 0);
    Ok(message)
}

fn dismiss(
    review: &mut ReviewState,
    name: &str,
    n: Option<usize>,
    owner: &Owner,
) -> Result<String, ActionError> {
    if review.verdict == Verdict::Running {
        return conflict(format!("the review of {name} is still running"));
    }
    let index = match n {
        Some(0) => return conflict("findings are numbered from 1".into()),
        Some(n) if n > review.findings.len() => {
            return conflict(format!(
                "{name} has {} finding(s); there is no finding {n}",
                review.findings.len()
            ))
        }
        Some(n) => n - 1,
        None => match review.findings.iter().position(|f| f.blocking) {
            Some(i) => i,
            None => return conflict(format!("{name} has no blocking finding to dismiss")),
        },
    };
    let finding = &mut review.findings[index];
    if let Some(done) = &finding.dismissed {
        return conflict(format!(
            "finding {} was already dismissed by {}",
            index + 1,
            done.by
        ));
    }
    finding.dismissed = Some(owner.clone());
    finding.blocking = false;
    Ok(format!(
        "dismissed finding {} on {name}: {} (by {})",
        index + 1,
        finding.title,
        owner.by
    ))
}

fn short(sha: &str) -> String {
    sha.chars().take(7).collect()
}

/// Keep what the owner did while a poll that started earlier ran: the same head's overrides.
pub(crate) fn carry_owner_edits(live: &Watch, next: &mut Watch) {
    let (Some(live), Some(next)) = (live.review.as_ref(), next.review.as_mut()) else {
        return;
    };
    if live.head_sha != next.head_sha {
        return;
    }
    if next.merge_anyway.is_none() {
        next.merge_anyway = live.merge_anyway.clone();
    }
    for (was, now) in live.findings.iter().zip(next.findings.iter_mut()) {
        if was.dismissed.is_some() && now.dismissed.is_none() && was.title == now.title {
            now.dismissed = was.dismissed.clone();
            now.blocking = false;
        }
    }
}

impl Registry {
    /// An owner action on a review; persisted at once.
    pub(crate) fn review_action(
        &self,
        session: Option<&str>,
        pr: Option<u64>,
        action: ReviewAction,
        by: &str,
        now: i64,
    ) -> Result<String, ActionError> {
        let mut guard = self.lock();
        let id = crate::serve_merge_queue::resolve(&guard, session, pr)?;
        let result = apply(&mut guard, &id, pr, action, by, now);
        self.persist(&guard);
        result
    }
}

#[derive(Deserialize)]
pub(super) struct ActionRequest {
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    pr: Option<u64>,
    /// `merge_anyway` | `dismiss`
    action: String,
    #[serde(default)]
    finding: Option<usize>,
    /// A label for the surface or person acting (`tui`, `phone`, `floris (cli)`).
    #[serde(default)]
    by: Option<String>,
}

pub(super) async fn review_action(
    State(state): State<Arc<DaemonState>>,
    Json(request): Json<ActionRequest>,
) -> Response {
    let action = match request.action.as_str() {
        "merge_anyway" => ReviewAction::MergeAnyway,
        "dismiss" => ReviewAction::Dismiss(request.finding),
        other => {
            return err_response(
                axum::http::StatusCode::BAD_REQUEST,
                &format!("unknown review action '{other}'; use merge_anyway or dismiss"),
            )
        }
    };
    let now = super::now_secs();
    let result = registry().review_action(
        request.session.as_deref(),
        request.pr,
        action,
        request.by.as_deref().unwrap_or_default(),
        now,
    );
    match result {
        Ok(message) => {
            state.registry.notify_fleet();
            json_response(&serde_json::json!({
                "ok": true,
                "message": message,
                "queues": crate::serve_merge_queue::snapshot(&registry().all()),
            }))
        }
        Err(ActionError::NotFound(m)) => err_response(axum::http::StatusCode::NOT_FOUND, &m),
        Err(ActionError::Conflict(m)) => err_response(axum::http::StatusCode::CONFLICT, &m),
    }
}
