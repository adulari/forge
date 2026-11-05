//! The daemon calls IDE mode's review tools make: files, blame, checks, shells, the Files tree,
//! commit messages, shipping and the Claude Code hand-off.

use forge_tui::board::ToastLevel;
use forge_tui::ide::checks::WireActions;
use forge_tui::ide::wire::{BlameResponse, WireEntries, WireFile, WireJobs, WireTail};
use forge_tui::ide::{IdeEvent, SaveFailure};

use super::{get, post, put, Api, CallError};

impl Api {
    pub(super) async fn file(&self, session: &str, path: &str) {
        let result = get::<WireFile>(
            &self.http,
            &self.base,
            &self.token,
            "api/workspace/file",
            &[("session", session), ("path", path)],
        )
        .await;
        self.send(IdeEvent::File {
            session_id: session.to_string(),
            path: path.to_string(),
            result: result.map_err(|e| e.message),
        });
    }

    /// Save with the hash the editor loaded; `None` overwrites, reading the current hash first.
    pub(super) async fn save(
        &self,
        session: &str,
        path: &str,
        content: String,
        expected: Option<String>,
    ) {
        let result = self.save_inner(session, path, content, expected).await;
        self.send(IdeEvent::Saved {
            session_id: session.to_string(),
            path: path.to_string(),
            result,
        });
    }

    pub(super) async fn save_inner(
        &self,
        session: &str,
        path: &str,
        content: String,
        expected: Option<String>,
    ) -> Result<String, SaveFailure> {
        let other = |e: CallError| SaveFailure::Other(e.message);
        let hash = match expected {
            Some(hash) => hash,
            None => {
                get::<WireFile>(
                    &self.http,
                    &self.base,
                    &self.token,
                    "api/workspace/file",
                    &[("session", session), ("path", path)],
                )
                .await
                .map_err(other)?
                .hash
            }
        };
        let body = serde_json::json!({
            "session": session,
            "path": path,
            "content": content,
            "expected_hash": hash,
        });
        match put(
            &self.http,
            &self.base,
            &self.token,
            "api/workspace/file",
            body,
        )
        .await
        {
            Ok(value) => Ok(value["hash"].as_str().unwrap_or_default().to_string()),
            Err(e) => Err(match e.status {
                409 => SaveFailure::Conflict,
                403 => SaveFailure::ReadOnly,
                413 => SaveFailure::TooLarge,
                _ => SaveFailure::Other(e.message),
            }),
        }
    }

    pub(super) async fn blame(&self, session: &str, path: &str) {
        let result = get::<BlameResponse>(
            &self.http,
            &self.base,
            &self.token,
            "api/blame",
            &[("session", session), ("path", path)],
        )
        .await;
        self.send(IdeEvent::Blame {
            session_id: session.to_string(),
            path: path.to_string(),
            result: result.map(|r| r.lines).map_err(|e| e.message),
        });
    }

    pub(super) async fn checks(&self, session: &str) {
        let result = get::<WireActions>(
            &self.http,
            &self.base,
            &self.token,
            "api/terminals/actions",
            &[("session", session)],
        )
        .await;
        self.send(IdeEvent::Checks(
            session.to_string(),
            result.map_err(|e| e.message),
        ));
    }

    pub(super) async fn jobs(&self, session: &str) {
        let result = get::<WireJobs>(
            &self.http,
            &self.base,
            &self.token,
            &format!("api/sessions/{session}/jobs"),
            &[],
        )
        .await;
        self.send(IdeEvent::Jobs(
            session.to_string(),
            result.map(|r| r.jobs).map_err(|e| e.message),
        ));
    }

    pub(super) async fn job_tail(&self, session: &str, job: u32) {
        let result = get::<WireTail>(
            &self.http,
            &self.base,
            &self.token,
            &format!("api/sessions/{session}/jobs/{job}/tail"),
            &[("lines", "300")],
        )
        .await;
        self.send(IdeEvent::JobTail {
            session_id: session.to_string(),
            job,
            result: result.map_err(|e| e.message),
        });
    }

    pub(super) async fn entries(&self, session: &str, dir: &str) {
        let result = get::<WireEntries>(
            &self.http,
            &self.base,
            &self.token,
            "api/workspace/entries",
            &[("session", session), ("path", dir)],
        )
        .await;
        self.send(IdeEvent::Entries {
            session_id: session.to_string(),
            dir: dir.to_string(),
            result: result.map_err(|e| e.message),
        });
    }

    pub(super) async fn suggest(&self, session: &str) {
        let result = post(
            &self.http,
            &self.base,
            &self.token,
            "api/git/commit-message",
            serde_json::json!({ "session": session }),
        )
        .await
        .map(|v| v["message"].as_str().unwrap_or_default().to_string())
        .map_err(|e| e.message);
        self.send(IdeEvent::Suggested(session.to_string(), result));
    }

    /// Stage, write a message, commit and push as asked; stops at the first failure.
    pub(super) async fn ship(
        &self,
        session: String,
        stage: Vec<String>,
        message: Option<String>,
        push: bool,
    ) {
        let run = async {
            if !stage.is_empty() {
                post(
                    &self.http,
                    &self.base,
                    &self.token,
                    "api/git/stage",
                    serde_json::json!({ "session": session, "paths": stage }),
                )
                .await
                .map_err(|e| format!("stage failed: {}", e.message))?;
            }
            let message = match message {
                Some(message) => message,
                None => post(
                    &self.http,
                    &self.base,
                    &self.token,
                    "api/git/commit-message",
                    serde_json::json!({ "session": session }),
                )
                .await
                .map_err(|e| format!("could not write a message: {}", e.message))?["message"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
            };
            if message.trim().is_empty() {
                return Err("no commit message".to_string());
            }
            post(
                &self.http,
                &self.base,
                &self.token,
                "api/git/commit",
                serde_json::json!({ "session": session, "message": message }),
            )
            .await
            .map_err(|e| format!("commit failed: {}", e.message))?;
            if push {
                post(
                    &self.http,
                    &self.base,
                    &self.token,
                    "api/git/push",
                    serde_json::json!({ "session": session }),
                )
                .await
                .map_err(|e| format!("committed, but the push failed: {}", e.message))?;
            }
            Ok(if push {
                "committed and pushed"
            } else {
                "committed"
            }
            .to_string())
        };
        self.send(IdeEvent::GitOp(run.await));
    }

    pub(super) async fn run_check(&self, session: &str, action: &str) -> Option<String> {
        match post(
            &self.http,
            &self.base,
            &self.token,
            "api/terminals/actions/run",
            serde_json::json!({ "session": session, "action": action }),
        )
        .await
        {
            Ok(value) => value["terminal_id"].as_str().map(str::to_string),
            Err(e) => {
                self.send(IdeEvent::Toast(ToastLevel::Error, e.message));
                None
            }
        }
    }

    pub(super) async fn continue_claude(&self, session: &str) {
        let result = post(
            &self.http,
            &self.base,
            &self.token,
            &format!("api/sessions/{session}/claude"),
            serde_json::json!({}),
        )
        .await
        .map_err(|e| {
            if matches!(e.status, 404 | 405) {
                "this daemon cannot hand a session to Claude Code yet: update and restart `forge serve`"
                    .to_string()
            } else {
                e.message
            }
        })
        .and_then(|v| {
            let cwd = v["cwd"].as_str().unwrap_or_default().to_string();
            let command = v["command"].as_str().unwrap_or_default().to_string();
            if command.is_empty() {
                Err("the daemon returned no command to run".to_string())
            } else {
                Ok((cwd, command))
            }
        });
        self.send(IdeEvent::Handoff(result));
    }
}
