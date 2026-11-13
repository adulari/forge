//! The daemon calls behind IDE mode's hunk actions: stage/unstage/revert one hunk, accept the
//! routine hunks of a group, undo an accept and discard a file.

use forge_tui::ide::hunk::UndoRecord;
use forge_tui::ide::wire::WireDiff;
use forge_tui::ide::{HunkFailure, HunkOutcome, IdeEvent};

use super::{classify, get, post, Api, CallError};

impl Api {
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn hunk(
        &self,
        session: String,
        path: String,
        header: String,
        patch: String,
        action: forge_tui::ide::hunk::HunkAction,
        undo: Option<UndoRecord>,
        is_undo: bool,
    ) {
        let body = serde_json::json!({
            "session": session,
            "path": path,
            "hunk_header": header,
            "patch": patch,
            "action": action.wire(),
        });
        let result = post(&self.http, &self.base, &self.token, "api/git/hunk", body).await;
        self.send(IdeEvent::Hunk(HunkOutcome {
            session_id: session,
            path,
            action,
            undo: result.is_ok().then_some(undo).flatten(),
            is_undo,
            result: result.map(|_| ()).map_err(classify),
        }));
    }

    /// Stage the routine hunks of each file: read its working-tree diff, then one `stage` request
    /// per routine hunk. A refused hunk is counted, never retried; the next changes read shows
    /// what is left.
    pub(super) async fn accept_routine(&self, targets: Vec<forge_tui::ide::RoutineTarget>) {
        let (mut staged, mut failed) = (0usize, 0usize);
        let mut records: Vec<UndoRecord> = Vec::new();
        for target in targets {
            let diff: Result<WireDiff, CallError> = get(
                &self.http,
                &self.base,
                &self.token,
                "api/git/diff",
                &[
                    ("session", target.session_id.as_str()),
                    ("path", target.path.as_str()),
                    ("staged", "false"),
                    ("context", "3"),
                ],
            )
            .await;
            let Ok(diff) = diff else {
                failed += 1;
                continue;
            };
            let Some(file) = diff.files.iter().find(|f| f.path == target.path) else {
                continue;
            };
            let mut done = Vec::new();
            for step in forge_tui::ide::hunk::routine_stage_steps(file) {
                let body = serde_json::json!({
                    "session": target.session_id,
                    "path": target.path,
                    "hunk_header": step.hunk_header,
                    "patch": step.patch,
                    "action": "stage",
                });
                match post(&self.http, &self.base, &self.token, "api/git/hunk", body).await {
                    Ok(_) => {
                        staged += 1;
                        done.push(step);
                    }
                    Err(_) => failed += 1,
                }
            }
            if !done.is_empty() {
                done.reverse();
                records.push(UndoRecord {
                    action: forge_tui::ide::hunk::HunkAction::Unstage,
                    ..UndoRecord::revert(
                        target.session_id,
                        target.path.clone(),
                        format!("routine hunks of {}", target.path),
                        done,
                    )
                });
            }
        }
        let mut records = records.into_iter();
        let undo = records.next().map(|mut first| {
            first.with = records.collect();
            first
        });
        self.send(IdeEvent::Routine(forge_tui::ide::RoutineOutcome {
            staged,
            failed,
            undo,
        }));
    }

    /// Take an accept back: unstage each recorded hunk, one request at a time. Reported as one
    /// undo outcome; the first refusal is the one the user sees.
    pub(super) async fn unstage_hunks(&self, records: Vec<UndoRecord>) {
        let mut result: Result<(), HunkFailure> = Ok(());
        let first = records
            .first()
            .map(|r| (r.session_id.clone(), r.path.clone()));
        for record in records {
            for step in record.steps {
                let body = serde_json::json!({
                    "session": record.session_id,
                    "path": record.path,
                    "hunk_header": step.hunk_header,
                    "patch": step.patch,
                    "action": "unstage",
                });
                if let Err(error) =
                    post(&self.http, &self.base, &self.token, "api/git/hunk", body).await
                {
                    if result.is_ok() {
                        result = Err(classify(error));
                    }
                }
            }
        }
        let (session_id, path) = first.unwrap_or_default();
        self.send(IdeEvent::Hunk(HunkOutcome {
            session_id,
            path,
            action: forge_tui::ide::hunk::HunkAction::Unstage,
            undo: None,
            is_undo: true,
            result,
        }));
    }

    pub(super) async fn discard(&self, session: String, path: String, undo: Option<UndoRecord>) {
        let body = serde_json::json!({ "session": session, "paths": [path] });
        let result = post(&self.http, &self.base, &self.token, "api/git/discard", body).await;
        self.send(IdeEvent::Hunk(HunkOutcome {
            session_id: session,
            path,
            action: forge_tui::ide::hunk::HunkAction::Revert,
            undo: result.is_ok().then_some(undo).flatten(),
            is_undo: false,
            result: result.map(|_| ()).map_err(classify),
        }));
    }
}
