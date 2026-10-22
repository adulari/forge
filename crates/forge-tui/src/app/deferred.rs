//! Results of slash commands that finish after the command returns.
//!
//! `/doctor`, `/diff` and `/mcp reconnect` run subprocesses or network connects that can take
//! seconds. Awaiting them in the dispatcher would freeze the render loop for that long, so the
//! work runs on its own task and posts its result to this inbox; the loop drains it every frame.

use std::sync::{Arc, Mutex};

use super::output_view::OutputView;
use super::App;

/// What a background command hands back to the chat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Deferred {
    /// Notice lines for the transcript.
    Notes(Vec<String>),
    /// A long text: the full-screen viewer when there is one, plain scrollback lines otherwise.
    /// `diff` colours unified-diff lines.
    View {
        title: String,
        summary: String,
        text: String,
        diff: bool,
    },
}

/// The chat's inbox for [`Deferred`] results. Clones share one queue, so a task can own one.
#[derive(Debug, Clone, Default)]
pub struct DeferredInbox(Arc<Mutex<Vec<Deferred>>>);

impl DeferredInbox {
    pub fn post(&self, item: Deferred) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).push(item);
    }

    fn take(&self) -> Vec<Deferred> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

impl App {
    /// A handle a background task posts its result through.
    pub fn deferred_inbox(&self) -> DeferredInbox {
        self.deferred.clone()
    }

    /// Show whatever background commands have finished. True when anything arrived.
    pub fn drain_deferred(&mut self) -> bool {
        let items = self.deferred.take();
        let arrived = !items.is_empty();
        for item in items {
            match item {
                Deferred::Notes(lines) => {
                    for line in lines {
                        self.note(&line);
                    }
                }
                Deferred::View {
                    title,
                    summary,
                    text,
                    diff,
                } => self.show_text(&title, &summary, &text, diff),
            }
        }
        arrived
    }

    /// Long command output: the searchable full-screen viewer, or plain scrollback lines inline.
    pub fn show_text(&mut self, title: &str, summary: &str, text: &str, diff: bool) {
        if self.fullscreen {
            self.output_view = Some(OutputView::text(title, summary, text, diff));
            return;
        }
        self.note(&format!("{title} · {summary}"));
        self.print_lines(text.lines().map(str::to_string).collect());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn posted_results_surface_once_on_the_next_drain() {
        let mut app = App::default();
        let inbox = app.deferred_inbox();
        assert!(!app.drain_deferred(), "nothing posted yet");
        inbox.post(Deferred::Notes(vec!["✓ reconnected github".into()]));
        assert!(app.drain_deferred());
        assert!(!app.drain_deferred(), "drained exactly once");
    }

    #[test]
    fn a_view_opens_the_viewer_in_fullscreen() {
        let mut app = App {
            fullscreen: true,
            ..App::default()
        };
        app.deferred_inbox().post(Deferred::View {
            title: "/diff".into(),
            summary: "1 file".into(),
            text: "+added\n-removed".into(),
            diff: true,
        });
        app.drain_deferred();
        assert!(app.output_view.is_some());
    }
}
