//! Idle Ctrl-C (the `interrupt` action): clears a non-empty draft, and on an empty input needs a
//! second press inside [`QUIT_DOUBLE_TAP`] to quit, so a stray keystroke never loses a prompt or
//! the session.

use super::*;
use crate::prompt_history::PromptHistory;

pub const QUIT_DOUBLE_TAP: std::time::Duration = std::time::Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleInterrupt {
    Cleared,
    Armed,
    Quit,
}

impl App {
    /// Handle an idle Interrupt press. A non-empty draft is saved to `history` (so ↑ recalls it)
    /// and cleared; clearing never arms the exit. On an empty input the first press arms and the
    /// second inside the window quits.
    pub fn idle_interrupt(&mut self, history: &mut PromptHistory) -> IdleInterrupt {
        if !self.input.is_empty() {
            let draft = self.resolved_draft();
            history.record(&draft);
            self.input.clear();
            self.input_cursor = 0;
            self.paste_blocks.clear();
            self.quit_armed = None;
            self.note("input cleared — ↑ to recall");
            return IdleInterrupt::Cleared;
        }
        let now = std::time::Instant::now();
        if self
            .quit_armed
            .is_some_and(|at| now.duration_since(at) < QUIT_DOUBLE_TAP)
        {
            self.quit_armed = None;
            return IdleInterrupt::Quit;
        }
        self.quit_armed = Some(now);
        IdleInterrupt::Armed
    }

    pub fn disarm_quit(&mut self) {
        self.quit_armed = None;
    }

    /// Drop an expired quit arm. Returns true when it did (the hint needs a redraw).
    pub fn expire_quit_hint(&mut self) -> bool {
        if self
            .quit_armed
            .is_some_and(|at| at.elapsed() >= QUIT_DOUBLE_TAP)
        {
            self.quit_armed = None;
            return true;
        }
        false
    }

    pub fn quit_hint_active(&self) -> bool {
        self.quit_armed
            .is_some_and(|at| at.elapsed() < QUIT_DOUBLE_TAP)
    }

    /// The draft as the user would have submitted it: text paste blocks expanded, image
    /// placeholders dropped, without consuming the blocks.
    fn resolved_draft(&self) -> String {
        let mut text = self.input.clone();
        for block in &self.paste_blocks {
            if let Some(pos) = text.find(&block.placeholder) {
                let span = pos..pos + block.placeholder.len();
                match &block.kind {
                    PasteKind::Text(content) => text.replace_range(span, content),
                    PasteKind::Image(_) => text.replace_range(span, ""),
                }
            }
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_with(input: &str) -> App {
        App {
            input: input.to_string(),
            input_cursor: input.len(),
            ..Default::default()
        }
    }

    #[test]
    fn text_is_saved_to_history_and_cleared_without_arming() {
        let mut app = app_with("half-typed prompt");
        let mut h = PromptHistory::new(vec![]);
        assert_eq!(app.idle_interrupt(&mut h), IdleInterrupt::Cleared);
        assert!(app.input.is_empty());
        assert_eq!(app.input_cursor, 0);
        assert_eq!(h.entries(), ["half-typed prompt"]);
        assert!(!app.quit_hint_active(), "clearing does not arm exit");
        let mut cur = 0;
        assert!(h.handle_key(&KeyKind::Up, &mut app.input, &mut cur));
        assert_eq!(app.input, "half-typed prompt");
    }

    #[test]
    fn clearing_dedupes_against_the_last_entry_and_expands_paste_blocks() {
        let mut app = app_with("same");
        let mut h = PromptHistory::new(vec!["same".into()]);
        app.idle_interrupt(&mut h);
        assert_eq!(h.entries(), ["same"]);

        let mut app = app_with("see [Pasted text #1]");
        app.paste_blocks.push(PasteBlock {
            placeholder: "[Pasted text #1]".into(),
            kind: PasteKind::Text("BODY".into()),
        });
        app.idle_interrupt(&mut h);
        assert_eq!(h.entries().last().unwrap(), "see BODY");
        assert!(app.paste_blocks.is_empty());
    }

    #[test]
    fn empty_input_arms_then_second_press_quits() {
        let mut app = App::default();
        let mut h = PromptHistory::default();
        assert_eq!(app.idle_interrupt(&mut h), IdleInterrupt::Armed);
        assert!(app.quit_hint_active());
        assert_eq!(app.idle_interrupt(&mut h), IdleInterrupt::Quit);
        assert!(!app.quit_hint_active());
    }

    #[test]
    fn text_then_two_more_presses_needed_to_quit() {
        let mut app = app_with("x");
        let mut h = PromptHistory::default();
        assert_eq!(app.idle_interrupt(&mut h), IdleInterrupt::Cleared);
        assert_eq!(app.idle_interrupt(&mut h), IdleInterrupt::Armed);
        assert_eq!(app.idle_interrupt(&mut h), IdleInterrupt::Quit);
    }

    #[test]
    fn another_key_disarms() {
        let mut app = App::default();
        let mut h = PromptHistory::default();
        app.idle_interrupt(&mut h);
        app.disarm_quit();
        assert_eq!(app.idle_interrupt(&mut h), IdleInterrupt::Armed);
    }

    #[test]
    fn window_expiry_disarms_and_requests_redraw() {
        let mut app = App::default();
        let mut h = PromptHistory::default();
        app.idle_interrupt(&mut h);
        assert!(!app.expire_quit_hint());
        app.quit_armed = Some(std::time::Instant::now() - QUIT_DOUBLE_TAP * 2);
        assert!(!app.quit_hint_active());
        assert!(app.expire_quit_hint());
        assert_eq!(app.idle_interrupt(&mut h), IdleInterrupt::Armed);
    }

    #[test]
    fn statusline_shows_the_quit_hint_while_armed() {
        let mut app = App::default();
        let mut h = PromptHistory::default();
        app.idle_interrupt(&mut h);
        assert_eq!(
            crate::app::render::status_line::statusline_hint(&app),
            "Press Ctrl+C again to exit"
        );
    }
}
