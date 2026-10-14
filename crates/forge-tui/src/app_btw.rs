//! The `/btw` overlay: a side question's answer shown in a dismissable card over the live view,
//! like Claude Code's, instead of being printed into the scrollback. The answer was never part of
//! the conversation, yet as scrollback it sat between real turns for the rest of the session.

use super::*;

/// One `/btw` exchange on screen. `answer: None` while the side call is still running.
#[derive(Debug, Clone, Default)]
pub struct BtwOverlay {
    pub question: String,
    pub answer: Option<String>,
    /// `model · $cost`, empty until answered.
    pub meta: String,
    pub failed: bool,
    pub scroll: u16,
}

impl App {
    /// Show the card for `question` in its waiting state.
    pub fn open_btw(&mut self, question: &str) {
        self.btw_dismissed = None;
        self.btw = Some(BtwOverlay {
            question: question.trim().to_string(),
            ..BtwOverlay::default()
        });
    }

    pub fn btw_open(&self) -> bool {
        self.btw.is_some()
    }

    /// Fill the card with the answer (opening one if the question was asked from elsewhere, e.g.
    /// a remote client).
    pub(crate) fn apply_btw_answer(
        &mut self,
        question: String,
        answer: String,
        model: String,
        cost_usd: Option<f64>,
    ) {
        let meta = match cost_usd {
            Some(c) if c > 0.0 => format!("{model} · ${c:.4}"),
            _ => model,
        };
        if self.btw_dismissed.as_deref() == Some(question.as_str()) {
            self.btw_dismissed = None;
            return;
        }
        let same = self.btw.as_ref().is_some_and(|o| o.question == question);
        if !same {
            self.open_btw(&question);
        }
        if let Some(card) = &mut self.btw {
            card.answer = Some(answer);
            card.meta = meta;
            card.failed = false;
            card.scroll = 0;
        }
    }

    /// Show a `/btw` failure in the waiting card. Returns false when no card was waiting, so the
    /// caller prints the warning normally.
    pub(crate) fn apply_btw_failure(&mut self, message: &str) -> bool {
        if self.btw_dismissed.is_some() && message.contains("/btw") {
            self.btw_dismissed = None;
            return true;
        }
        match &mut self.btw {
            Some(card) if card.answer.is_none() && message.contains("/btw") => {
                card.answer = Some(message.to_string());
                card.failed = true;
                true
            }
            _ => false,
        }
    }

    /// Route a key to the open card. Esc, Enter and Space close it; ↑/↓ and PgUp/PgDn scroll a
    /// long answer. Every key is consumed while the card is open. Closing a card that is still
    /// waiting remembers its question so the late answer stays closed: one Esc closes the card,
    /// the next Esc reaches the turn interrupt.
    pub fn btw_key(&mut self, key: &KeyKind) {
        let Some(card) = &mut self.btw else { return };
        match key {
            KeyKind::Esc | KeyKind::Enter | KeyKind::Char(' ') => {
                if card.answer.is_none() {
                    self.btw_dismissed = Some(card.question.clone());
                }
                self.btw = None;
            }
            KeyKind::Up => card.scroll = card.scroll.saturating_sub(1),
            KeyKind::Down => card.scroll = card.scroll.saturating_add(1),
            KeyKind::PageUp => card.scroll = card.scroll.saturating_sub(10),
            KeyKind::PageDown => card.scroll = card.scroll.saturating_add(10),
            _ => {}
        }
    }

    /// The card as plain text for the remote overlay projection.
    pub(crate) fn btw_overlay_body(card: &BtwOverlay) -> String {
        let answer = card.answer.as_deref().unwrap_or("thinking…");
        if card.meta.is_empty() {
            format!("{}\n\n{answer}", card.question)
        } else {
            format!(
                "{}\n\n{answer}\n\n({}, not part of the conversation)",
                card.question, card.meta
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_answer_fills_the_card_and_never_reaches_the_scrollback() {
        let mut app = App::default();
        app.open_btw("what is EAGAIN?");
        app.apply(PresenterEvent::BtwAnswer {
            question: "what is EAGAIN?".into(),
            answer: "Try again later.".into(),
            model: "ollama::m".into(),
            cost_usd: Some(0.0012),
        });
        let card = app.btw.as_ref().expect("card stays open until dismissed");
        assert_eq!(card.answer.as_deref(), Some("Try again later."));
        assert_eq!(card.meta, "ollama::m · $0.0012");
        assert!(
            app.drain_flush().is_empty(),
            "nothing printed into the chat"
        );
    }

    #[test]
    fn esc_enter_and_space_dismiss_and_arrows_scroll() {
        for key in [KeyKind::Esc, KeyKind::Enter, KeyKind::Char(' ')] {
            let mut app = App::default();
            app.open_btw("q");
            app.btw_key(&key);
            assert!(!app.btw_open(), "{key:?} closes the card");
        }
        let mut app = App::default();
        app.open_btw("q");
        app.btw_key(&KeyKind::Down);
        app.btw_key(&KeyKind::Down);
        app.btw_key(&KeyKind::Up);
        assert_eq!(app.btw.as_ref().unwrap().scroll, 1);
        app.btw_key(&KeyKind::Char('x'));
        assert!(app.btw_open(), "other keys do not dismiss");
    }

    #[test]
    fn a_failure_lands_in_the_waiting_card_not_the_chat() {
        let mut app = App::default();
        app.open_btw("q");
        app.apply(PresenterEvent::Warning("/btw failed: timeout".into()));
        let card = app.btw.as_ref().unwrap();
        assert!(card.failed);
        assert_eq!(card.answer.as_deref(), Some("/btw failed: timeout"));
        assert!(app.drain_flush().is_empty());
    }

    #[test]
    fn an_unrelated_warning_still_prints_while_the_card_waits() {
        let mut app = App::default();
        app.open_btw("q");
        app.apply(PresenterEvent::Warning("disk almost full".into()));
        assert!(app.btw.as_ref().unwrap().answer.is_none());
        assert!(!app.drain_flush().is_empty());
    }

    #[test]
    fn a_card_dismissed_while_waiting_stays_closed_when_the_answer_lands() {
        let mut app = App::default();
        app.open_btw("why?");
        app.btw_key(&KeyKind::Esc);
        assert!(!app.btw_open());
        app.apply(PresenterEvent::BtwAnswer {
            question: "why?".into(),
            answer: "because".into(),
            model: "m".into(),
            cost_usd: None,
        });
        assert!(
            !app.btw_open(),
            "a late answer must not reopen the card and steal the next Esc"
        );
        app.apply(PresenterEvent::BtwAnswer {
            question: "other".into(),
            answer: "a remote side question still opens".into(),
            model: "m".into(),
            cost_usd: None,
        });
        assert!(app.btw_open());
    }

    #[test]
    fn a_late_failure_for_a_dismissed_card_is_swallowed() {
        let mut app = App::default();
        app.open_btw("q");
        app.btw_key(&KeyKind::Esc);
        app.apply(PresenterEvent::Warning("/btw failed: timeout".into()));
        assert!(!app.btw_open());
        assert!(app.drain_flush().is_empty());
    }
}
