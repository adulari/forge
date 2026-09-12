//! Performing a [`BoardAction`]. The board decides WHAT should happen; this decides how, and it
//! is the only place in the board that writes anything to the daemon.
//!
//! Each action either goes down a session's WebSocket (the things a session's own driver must see
//! in order — prompts, steers, permission answers), or over HTTP (the things the daemon owns:
//! interrupt, archive, mode, create, resume). HTTP actions run as detached tasks so a slow daemon
//! can never stall the render loop, and each reports itself with a toast.

use forge_tui::board::{BoardAction, BoardEvent, ToastLevel};
use tokio::sync::mpsc;

use super::client::{api_url, create_session, fetch_detail, post_action};
use super::sockets::SocketPool;
use super::{short_id, Ev};

/// Titles are a card's headline, not a transcript.
const TITLE_MAX_CHARS: usize = 60;

/// What the render loop must do after an action — the two things this module cannot do itself,
/// because they need the terminal.
pub(crate) enum After {
    None,
    Quit,
    Attach(String),
}

pub(crate) struct Host<'a> {
    pub(crate) http: &'a reqwest::Client,
    pub(crate) base: &'a str,
    pub(crate) token: &'a str,
    pub(crate) ev: &'a mpsc::UnboundedSender<Ev>,
    pub(crate) refresh: &'a mpsc::UnboundedSender<()>,
    pub(crate) pool: &'a mut SocketPool,
    pub(crate) clipboard: &'a mut Option<arboard::Clipboard>,
}

pub(crate) fn perform(action: BoardAction, host: &mut Host<'_>) -> After {
    match action {
        BoardAction::Quit => return After::Quit,
        BoardAction::Attach(id) => return After::Attach(id),
        BoardAction::Input(id, json) => host.pool.send(&id, &json),
        BoardAction::Interrupt(id) => host.post(
            &format!("api/sessions/{id}/interrupt"),
            serde_json::json!({}),
            format!("interrupted {}", short_id(&id)),
        ),
        BoardAction::Archive(id) => host.post(
            &format!("api/sessions/{id}/archive"),
            serde_json::json!({}),
            format!("archived {}", short_id(&id)),
        ),
        BoardAction::SetMode(id, mode) => host.post(
            &format!("api/sessions/{id}/mode"),
            serde_json::json!({ "mode": mode }),
            format!("{} is now in {mode} mode", short_id(&id)),
        ),
        // Resume deliberately sends nothing but the id: the daemon restores the session's RECORDED
        // workspace, model pin and worktree from the store. A `cwd` here would be ignored, and a
        // `worktree` would be rejected outright.
        BoardAction::Resume(id) => host.post(
            "api/sessions",
            serde_json::json!({ "resume": id }),
            format!("resumed {}", short_id(&id)),
        ),
        BoardAction::NewSession {
            cwd,
            worktree,
            prompt,
        } => {
            let body = new_session_body(&cwd, worktree, &prompt);
            tokio::spawn(create_session(
                host.http.clone(),
                api_url(host.base, host.token, "api/sessions"),
                body,
                prompt,
                host.ev.clone(),
                host.refresh.clone(),
            ));
        }
        BoardAction::WantDetail(id) => {
            tokio::spawn(fetch_detail(
                host.http.clone(),
                host.base.to_string(),
                host.token.to_string(),
                id,
                host.ev.clone(),
            ));
        }
        BoardAction::Refresh => {
            let _ = host.refresh.send(());
        }
        BoardAction::Copy(text) => {
            copy(host.clipboard, &text);
            let _ = host.ev.send(Ev::Board(BoardEvent::Toast(
                ToastLevel::Ok,
                "copied".to_string(),
            )));
        }
    }
    After::None
}

impl Host<'_> {
    fn post(&self, path: &str, body: serde_json::Value, ok: String) {
        tokio::spawn(post_action(
            self.http.clone(),
            api_url(self.base, self.token, path),
            body,
            ok,
            self.ev.clone(),
            self.refresh.clone(),
        ));
    }
}

/// `POST /api/sessions` for a new session. The daemon rejects unknown fields, so this sends only
/// what it declares — and gives the card a title up front, because an untitled card is
/// indistinguishable from every other untitled card until the first turn finishes.
pub(crate) fn new_session_body(cwd: &str, worktree: bool, prompt: &str) -> serde_json::Value {
    let title: String = prompt
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(TITLE_MAX_CHARS)
        .collect();
    serde_json::json!({ "cwd": cwd, "worktree": worktree, "title": title })
}

/// The system clipboard when there is one, and an OSC 52 sequence always: over SSH or in a
/// container `arboard` has nothing to talk to, and the escape makes the user's own terminal do the
/// copy instead.
fn copy(clipboard: &mut Option<arboard::Clipboard>, text: &str) {
    if let Some(cb) = clipboard.as_mut() {
        let _ = cb.set_text(text.to_owned());
    }
    osc52(text);
}

fn osc52(text: &str) {
    use base64::Engine as _;
    use std::io::Write as _;
    let payload = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
    let term = std::env::var("TERM").unwrap_or_default();
    let sequence = if term.starts_with("tmux") || term.starts_with("screen") {
        // Multiplexer passthrough, with each ESC doubled, so it reaches the outer terminal.
        format!("\x1bPtmux;\x1b\x1b]52;c;{payload}\x07\x1b\\")
    } else {
        format!("\x1b]52;c;{payload}\x07")
    };
    let mut out = std::io::stdout();
    let _ = out.write_all(sequence.as_bytes());
    let _ = out.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_session_carries_only_fields_the_daemon_declares() {
        let body = new_session_body("/repo", true, "make the parser stop panicking");
        // `POST /api/sessions` is `deny_unknown_fields`; an extra key is a 400, not a warning.
        let object = body.as_object().unwrap();
        assert_eq!(object.len(), 3);
        assert_eq!(body["cwd"], "/repo");
        assert_eq!(body["worktree"], true);
        assert_eq!(body["title"], "make the parser stop panicking");
    }

    #[test]
    fn a_long_first_prompt_becomes_a_card_sized_title() {
        let prompt = "word ".repeat(50);
        let body = new_session_body("/repo", false, &prompt);
        let title = body["title"].as_str().unwrap();
        assert_eq!(title.chars().count(), TITLE_MAX_CHARS);
    }

    #[test]
    fn a_multiline_prompt_does_not_smuggle_newlines_into_the_title() {
        let body = new_session_body("/repo", false, "fix this\n\nand also that");
        assert_eq!(body["title"], "fix this and also that");
    }
}
