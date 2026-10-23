//! A pinned model's context-window lookup that runs beside startup instead of in front of it.
//!
//! The windows live in the store, and the session reads them from there each time it sizes a
//! request, so a lookup that lands later is applied by the next request on its own. What must not
//! happen is a FIRST request sized before the lookup settled (that is the 32k-floor compaction the
//! lookup exists to prevent), so the turn waits for it right before sizing that request. By then a
//! person has typed a prompt and the wait is zero; a one-shot run waits only for what the routing
//! and recall work did not already cover.

use tokio::sync::watch;

use crate::{PresenterEvent, Session};

/// Receiving end of a background window lookup: `None` until it settles, then the pinned models
/// whose window is still unknown.
#[derive(Clone)]
pub struct WindowLookup(watch::Receiver<Option<Vec<String>>>);

impl WindowLookup {
    pub fn channel() -> (watch::Sender<Option<Vec<String>>>, Self) {
        let (tx, rx) = watch::channel(None);
        (tx, Self(rx))
    }

    async fn settled(mut self) -> Vec<String> {
        loop {
            if let Some(unknown) = self.0.borrow().clone() {
                return unknown;
            }
            if self.0.changed().await.is_err() {
                return Vec::new();
            }
        }
    }
}

impl Session {
    /// Attach the lookup started at session build; the first request waits for it.
    pub fn set_window_lookup(&mut self, lookup: Option<WindowLookup>) {
        self.window_lookup = lookup;
    }

    /// Wait for the startup window lookup (once), then say so when a window is still unknown.
    pub(crate) async fn settle_window_lookup(&mut self) {
        let Some(lookup) = self.window_lookup.take() else {
            return;
        };
        let unknown = lookup.settled().await;
        if unknown.is_empty() {
            return;
        }
        let notice = format!(
            "context window of {} is unknown — requests and auto-compaction are sized for {} \
             tokens; set `[mesh.context_windows]` to the real window",
            unknown.join(", "),
            forge_mesh::pricing::CONSERVATIVE_CONTEXT_WINDOW
        );
        // A pin startup already called unknown does not also need its window explained: that is a
        // second warning about a model the person is about to be sent to /model over.
        let already_flagged = unknown.iter().all(|id| self.unverified_models.contains(id));
        if self.presenter.is_attended() && !already_flagged {
            self.presenter.emit(PresenterEvent::Warning(notice));
        } else {
            tracing::warn!("{notice}");
        }
    }
}
