//! Hosting a subagent's permission question on the parent's own prompt.
//!
//! The parent turn is blocked inside its `spawn_agents` call, so its presenter is free to ask. The
//! children run on other tasks and reach it through a channel; this loop answers them one at a time,
//! which is what serializes the prompts of concurrent siblings.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use forge_types::{ConfirmOutcome, Presenter, PresenterEvent};
use tokio::sync::{mpsc, oneshot};

use crate::permission::{confirm_label, is_public_expose};
use crate::subagent::{ChildAnswer, ChildAsker, ChildQuestion};

pub(crate) type Pending = (ChildQuestion, oneshot::Sender<ChildAnswer>);

/// An asker that forwards each question to the returned receiver and waits for the reply.
pub(crate) fn channel_asker() -> (ChildAsker, mpsc::UnboundedReceiver<Pending>) {
    let (tx, rx) = mpsc::unbounded_channel::<Pending>();
    let asker: ChildAsker = Arc::new(move |question| {
        let tx = tx.clone();
        Box::pin(async move {
            let (reply, answer) = oneshot::channel();
            if tx.send((question, reply)).is_err() {
                return ChildAnswer::Deny(None);
            }
            answer.await.unwrap_or(ChildAnswer::Deny(None))
        })
    });
    (asker, rx)
}

/// Tools the user has allowed for the rest of this `spawn_agents` call.
#[derive(Default)]
pub(crate) struct Granted(HashSet<String>);

/// Put one child's question to the presenter. The call is announced first so the prompt has the
/// path or command next to it, and closed afterwards so no card is left spinning.
pub(crate) fn answer(
    presenter: &Mutex<&mut Box<dyn Presenter>>,
    granted: &mut Granted,
    question: &ChildQuestion,
) -> ChildAnswer {
    if granted.0.contains(&question.tool) {
        return ChildAnswer::Allow;
    }
    let Ok(mut guard) = presenter.lock() else {
        return ChildAnswer::Deny(None);
    };
    let presenter: &mut dyn Presenter = &mut ***guard;
    let what =
        confirm_label(&question.tool, &question.args).unwrap_or_else(|| question.tool.clone());
    let label = format!("{}: {what}", question.child);
    presenter.emit(PresenterEvent::ToolStart {
        name: label.clone(),
        args: question.args.to_string(),
    });
    let reply = presenter.confirm_reply(&label, question.side_effect);
    let allowed = reply.outcome.allows();
    presenter.emit(PresenterEvent::ToolResult {
        name: label,
        ok: allowed,
        summary: if allowed { "approved" } else { "declined" }.to_string(),
        detail: None,
    });
    if !allowed {
        return ChildAnswer::Deny(reply.note);
    }
    let standing = matches!(
        reply.outcome,
        ConfirmOutcome::AlwaysAllow | ConfirmOutcome::AllowSession
    );
    if standing && question.tool != "shell" && !is_public_expose(&question.tool, &question.args) {
        granted.0.insert(question.tool.clone());
    }
    ChildAnswer::Allow
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_types::{QChoice, SideEffect};

    struct Scripted {
        replies: Vec<ConfirmOutcome>,
        asked: Vec<String>,
        events: Vec<String>,
    }

    impl Presenter for Scripted {
        fn emit(&mut self, event: PresenterEvent) {
            if let PresenterEvent::ToolStart { name, .. } = event {
                self.events.push(name);
            }
        }
        fn confirm(&mut self, tool: &str, _: SideEffect) -> ConfirmOutcome {
            self.asked.push(tool.to_string());
            self.replies.remove(0)
        }
        fn ask(&mut self, _: &str, _: &[QChoice], _: bool) -> String {
            String::new()
        }
        fn read_line(&mut self) -> Option<String> {
            None
        }
    }

    fn question(tool: &str) -> ChildQuestion {
        ChildQuestion {
            child: "general-purpose-ab12".into(),
            tool: tool.into(),
            side_effect: SideEffect::Write,
            args: serde_json::json!({"path": "a.txt"}),
        }
    }

    #[test]
    fn the_prompt_names_the_child_and_a_session_grant_covers_later_calls() {
        let mut boxed: Box<dyn Presenter> = Box::new(Scripted {
            replies: vec![ConfirmOutcome::AllowSession, ConfirmOutcome::Deny],
            asked: Vec::new(),
            events: Vec::new(),
        });
        let presenter = Mutex::new(&mut boxed);
        let mut granted = Granted::default();
        assert_eq!(
            answer(&presenter, &mut granted, &question("write_file")),
            ChildAnswer::Allow
        );
        assert_eq!(
            answer(&presenter, &mut granted, &question("write_file")),
            ChildAnswer::Allow,
            "second write is covered by the session grant, not asked again"
        );
        assert_eq!(
            answer(&presenter, &mut granted, &question("edit_file")),
            ChildAnswer::Deny(None)
        );
    }

    #[test]
    fn a_shell_grant_is_never_widened_to_later_commands() {
        let mut boxed: Box<dyn Presenter> = Box::new(Scripted {
            replies: vec![ConfirmOutcome::AllowSession, ConfirmOutcome::Deny],
            asked: Vec::new(),
            events: Vec::new(),
        });
        let presenter = Mutex::new(&mut boxed);
        let mut granted = Granted::default();
        assert_eq!(
            answer(&presenter, &mut granted, &question("shell")),
            ChildAnswer::Allow
        );
        assert_eq!(
            answer(&presenter, &mut granted, &question("shell")),
            ChildAnswer::Deny(None)
        );
    }
}
