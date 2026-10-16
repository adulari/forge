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
    inner: Arc<Mutex<Inbox>>,
}

#[derive(Default, Debug)]
struct Inbox {
    pending: VecDeque<String>,
    /// Texts the model loop took since the last [`SteerInbox::clear`] / [`SteerInbox::settle`].
    /// Recorded under the same lock as the drain, so a surface settling the inbox sees every text
    /// the turn absorbed even when the turn's `Steered` event is still in flight.
    delivered: Vec<String>,
}

impl SteerInbox {
    /// Queue `text` for the running turn. A blank text is ignored.
    pub fn push(&self, text: impl Into<String>) {
        let text = text.into();
        if text.trim().is_empty() {
            return;
        }
        if let Ok(mut q) = self.inner.lock() {
            q.pending.push_back(text);
        }
    }

    /// Like [`Self::push`] but delivered before everything already waiting: an explicit steer
    /// outranks queued follow-ups, matching where the queue display puts it.
    pub fn push_front(&self, text: impl Into<String>) {
        let text = text.into();
        if text.trim().is_empty() {
            return;
        }
        if let Ok(mut q) = self.inner.lock() {
            q.pending.push_front(text);
        }
    }

    /// Withdraw the oldest not-yet-taken `text`; false when the turn already took it.
    pub fn remove_pending(&self, text: &str) -> bool {
        self.inner.lock().is_ok_and(|mut q| {
            q.pending
                .iter()
                .position(|t| t == text)
                .is_some_and(|i| q.pending.remove(i).is_some())
        })
    }

    /// Take every queued text, oldest first, recording each as delivered.
    pub fn drain(&self) -> Vec<String> {
        self.inner
            .lock()
            .map(|mut q| {
                let texts: Vec<String> = q.pending.drain(..).collect();
                q.delivered.extend(texts.iter().cloned());
                texts
            })
            .unwrap_or_default()
    }

    pub fn is_empty(&self) -> bool {
        self.inner
            .lock()
            .map(|q| q.pending.is_empty())
            .unwrap_or(true)
    }

    pub fn clear(&self) {
        if let Ok(mut q) = self.inner.lock() {
            q.pending.clear();
            q.delivered.clear();
        }
    }

    /// Close out a turn from the surface side: drop every text not yet delivered (the surface
    /// still holds it in its queue) and return those the turn already absorbed. Atomic with
    /// [`drain`](Self::drain), so a turn being aborted cannot absorb a text after this returns
    /// without it showing up here.
    pub fn settle(&self) -> Vec<String> {
        self.inner
            .lock()
            .map(|mut q| {
                q.pending.clear();
                std::mem::take(&mut q.delivered)
            })
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_front_is_delivered_before_queued_texts() {
        let inbox = SteerInbox::default();
        inbox.push("follow-up");
        inbox.push_front("steer");
        assert_eq!(inbox.drain(), ["steer", "follow-up"]);
    }

    #[test]
    fn a_withdrawn_text_is_never_drained() {
        let inbox = SteerInbox::default();
        inbox.push("a");
        inbox.push("b");
        assert!(inbox.remove_pending("a"));
        assert!(!inbox.remove_pending("a"));
        assert_eq!(inbox.drain(), ["b"]);
    }

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

    #[test]
    fn settle_returns_delivered_and_drops_pending() {
        let inbox = SteerInbox::default();
        inbox.push("absorbed");
        assert_eq!(inbox.drain(), vec!["absorbed".to_string()]);
        inbox.push("still queued");
        assert_eq!(inbox.settle(), vec!["absorbed".to_string()]);
        assert!(
            inbox.is_empty(),
            "pending texts stay with the surface's queue"
        );
        assert!(inbox.settle().is_empty(), "delivered is taken once");
        inbox.push("x");
        inbox.drain();
        inbox.clear();
        assert!(
            inbox.settle().is_empty(),
            "a new turn forgets earlier deliveries"
        );
    }
}
