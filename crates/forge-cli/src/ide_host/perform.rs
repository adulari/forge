//! Performing the actions IDE mode asks for: each one becomes a daemon call or a socket command.

use tokio::sync::mpsc;

use forge_tui::ide::IdeAction;

use super::{
    post, save_marks, save_prefs, stream_check, term_task, Api, Runtime, TermCmd, TermLink,
};
use crate::board::sockets::SocketPool;

pub(super) fn perform(
    action: IdeAction,
    api: &Api,
    pool: &mut SocketPool,
    refresh: &mpsc::UnboundedSender<()>,
    runtime: &mut Runtime,
) {
    let api = api.clone();
    match action {
        IdeAction::Close => {}
        IdeAction::RefreshChanges => {
            let _ = refresh.send(());
        }
        IdeAction::FetchDiff {
            session_id,
            path,
            worktree,
        } => {
            tokio::spawn(async move { api.diff(&session_id, &path, worktree).await });
        }
        IdeAction::FetchGitStatus(id) => {
            tokio::spawn(async move { api.status(&id).await });
        }
        IdeAction::FetchHistory(id) => {
            tokio::spawn(async move { api.history(&id).await });
        }
        IdeAction::Hunk {
            session_id,
            path,
            header,
            patch,
            action,
            undo,
            is_undo,
        } => {
            tokio::spawn(async move {
                api.hunk(session_id, path, header, patch, action, undo, is_undo)
                    .await
            });
        }
        IdeAction::AcceptRoutine(targets) => {
            tokio::spawn(async move { api.accept_routine(targets).await });
        }
        IdeAction::UnstageHunks(records) => {
            tokio::spawn(async move { api.unstage_hunks(records).await });
        }
        IdeAction::DiscardFile {
            session_id,
            path,
            undo,
        } => {
            tokio::spawn(async move { api.discard(session_id, path, undo).await });
        }
        IdeAction::StagePaths {
            session_id,
            paths,
            stage,
        } => {
            tokio::spawn(async move {
                let route = if stage {
                    "api/git/stage"
                } else {
                    "api/git/unstage"
                };
                let note = format!(
                    "{} {}",
                    if stage { "staged" } else { "unstaged" },
                    paths.join(", ")
                );
                api.git_op(
                    route,
                    serde_json::json!({ "session": session_id, "paths": paths }),
                    note,
                )
                .await;
            });
        }
        IdeAction::Commit {
            session_id,
            message,
        } => {
            tokio::spawn(async move {
                api.git_op(
                    "api/git/commit",
                    serde_json::json!({ "session": session_id, "message": message }),
                    "committed".to_string(),
                )
                .await;
            });
        }
        IdeAction::Push { session_id } => {
            tokio::spawn(async move {
                api.git_op(
                    "api/git/push",
                    serde_json::json!({ "session": session_id }),
                    "pushed".to_string(),
                )
                .await;
            });
        }
        IdeAction::Input(id, json) => pool.send(&id, &json),
        IdeAction::Watch(id) => pool.sync(&id.into_iter().collect::<Vec<_>>()),
        IdeAction::SaveReviewed(json) => {
            tokio::task::spawn_blocking(move || save_marks(&json));
        }
        IdeAction::SavePrefs(json) => {
            tokio::task::spawn_blocking(move || save_prefs(&json));
        }
        IdeAction::FetchFile { session_id, path } => {
            tokio::spawn(async move { api.file(&session_id, &path).await });
        }
        IdeAction::SaveFile {
            session_id,
            path,
            content,
            expected_hash,
        } => {
            tokio::spawn(async move { api.save(&session_id, &path, content, expected_hash).await });
        }
        IdeAction::ExternalEdit { .. } => {}
        IdeAction::FetchBlame { session_id, path } => {
            tokio::spawn(async move { api.blame(&session_id, &path).await });
        }
        IdeAction::FetchChecks(session) => {
            tokio::spawn(async move { api.checks(&session).await });
        }
        IdeAction::RunCheck { session_id, action } => {
            let key = (session_id.clone(), action.clone());
            if let Some(old) = runtime.checks.remove(&key) {
                old.abort();
            }
            let handle = tokio::spawn(async move {
                let Some(terminal_id) = api.run_check(&session_id, &action).await else {
                    return;
                };
                api.checks(&session_id).await;
                stream_check(&api, &session_id, &action, &terminal_id).await;
            });
            runtime.checks.insert(key, handle);
        }
        IdeAction::StopCheck { session_id, action } => {
            tokio::spawn(async move {
                let _ = post(
                    &api.http,
                    &api.base,
                    &api.token,
                    "api/terminals/actions/stop",
                    serde_json::json!({ "session": session_id, "action": action }),
                )
                .await;
                api.checks(&session_id).await;
            });
        }
        IdeAction::StreamCheck {
            session_id,
            action,
            terminal_id,
        } => {
            let key = (session_id.clone(), action.clone());
            if let Some(old) = runtime.checks.remove(&key) {
                old.abort();
            }
            let handle = tokio::spawn(async move {
                stream_check(&api, &session_id, &action, &terminal_id).await;
            });
            runtime.checks.insert(key, handle);
        }
        IdeAction::FetchJobs(session) => {
            tokio::spawn(async move { api.jobs(&session).await });
        }
        IdeAction::FetchJobTail { session_id, job } => {
            tokio::spawn(async move { api.job_tail(&session_id, job).await });
        }
        IdeAction::FetchEntries { session_id, dir } => {
            tokio::spawn(async move { api.entries(&session_id, &dir).await });
        }
        IdeAction::SuggestMessage(session) => {
            tokio::spawn(async move { api.suggest(&session).await });
        }
        IdeAction::Ship {
            session_id,
            stage,
            message,
            push,
        } => {
            tokio::spawn(async move { api.ship(session_id, stage, message, push).await });
        }
        IdeAction::ContinueClaude(session) => {
            tokio::spawn(async move { api.continue_claude(&session).await });
        }
        IdeAction::TermOpen {
            session_id,
            cols,
            rows,
        } => {
            let restart = runtime.term.is_some();
            runtime.close_terminal();
            let (cmds, rx) = mpsc::unbounded_channel();
            let task = tokio::spawn(term_task(
                api.base.clone(),
                api.token.clone(),
                session_id,
                (cols, rows),
                restart,
                api.ev.clone(),
                rx,
            ));
            runtime.term = Some(TermLink { cmds, task });
        }
        IdeAction::TermInput(bytes) => runtime.term_cmd(TermCmd::Input(bytes)),
        IdeAction::TermResize(cols, rows) => runtime.term_cmd(TermCmd::Resize(cols, rows)),
        IdeAction::TermClose => runtime.term_cmd(TermCmd::Close),
    }
}
