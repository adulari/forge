//! Mid-turn steering: prompts the user queues while a turn is running.
//!
//! The session is locked by the turn task for the whole turn, so a surface cannot call into it to
//! add a message. Instead it holds a clone of this inbox and pushes into it; the model loop drains
//! it at every boundary where a new user message is legal — after a tool step's results are in,
//! and where a response would otherwise have ended the turn — persists each text as a real user
//! message, and emits [`PresenterEvent::Steered`](forge_types::PresenterEvent::Steered) so the
//! surface can echo it in place and drop it from its pending list. A prompt still in the inbox
//! when the turn ends is NOT lost: the surface still holds it and starts the next turn with it,
//! exactly as the queue always worked; the inbox is cleared at the start of every turn so such a
//! prompt cannot be delivered twice.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default, Debug)]
pub struct SteerInbox {
    inner: Arc<Mutex<VecDeque<String>>>,
}

impl SteerInbox {
    /// Queue `text` for the running turn. A blank text is ignored.
    pub fn push(&self, text: impl Into<String>) {
        let text = text.into();
        if text.trim().is_empty() {
            return;
        }
        if let Ok(mut q) = self.inner.lock() {
            q.push_back(text);
        }
    }

    /// Take every queued text, oldest first.
    pub fn drain(&self) -> Vec<String> {
        self.inner
            .lock()
            .map(|mut q| q.drain(..).collect())
            .unwrap_or_default()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.lock().map(|q| q.is_empty()).unwrap_or(true)
    }

    pub fn clear(&self) {
        if let Ok(mut q) = self.inner.lock() {
            q.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drains_in_order_and_ignores_blank() {
        let inbox = SteerInbox::default();
        let other = inbox.clone();
        other.push("first");
        other.push("   ");
        other.push("second");
        assert!(!inbox.is_empty());
        assert_eq!(
            inbox.drain(),
            vec!["first".to_string(), "second".to_string()]
        );
        assert!(inbox.is_empty());
        assert!(inbox.drain().is_empty());
    }
}
