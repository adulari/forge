//! ↑/↓ prompt recall. One store per surface, seeded from the session's persisted user messages
//! (this session's prompts after recent prompts from other sessions in the same workspace). Every
//! prompt the user submits is recorded at the moment it is submitted, whether the session is idle
//! or busy: a prompt typed mid-turn may be steered into the running turn, run later as its own
//! turn, or be cut short by an interrupt, and recall must not depend on which of those happens.

use crate::app::{input_cursor_up, KeyKind};

#[derive(Debug, Default, Clone)]
pub struct PromptHistory {
    entries: Vec<String>,
    pos: Option<usize>,
    draft: String,
}

impl PromptHistory {
    pub fn new(entries: Vec<String>) -> Self {
        Self {
            entries,
            ..Self::default()
        }
    }

    pub fn entries(&self) -> &[String] {
        &self.entries
    }

    /// Record a submitted prompt (trimmed; blank and back-to-back duplicates skipped) and end any
    /// browse in progress.
    pub fn record(&mut self, line: &str) {
        self.pos = None;
        let line = line.trim();
        if !line.is_empty() && self.entries.last().map(String::as_str) != Some(line) {
            self.entries.push(line.to_string());
        }
    }

    /// The draft was edited: the next ↑ starts from the newest entry again.
    pub fn end_browse(&mut self) {
        self.pos = None;
    }

    /// Handle ↑/↓ for the input line. Returns false for any other key. ↑ on a lower row of a
    /// multiline draft moves the cursor up a row instead of recalling; ↑ on the first row steps
    /// back through history (saving the draft on the first step); ↓ steps forward and restores the
    /// draft past the newest entry.
    pub fn handle_key(&mut self, key: &KeyKind, input: &mut String, cursor: &mut usize) -> bool {
        match key {
            KeyKind::Up => {
                if let Some(up) = input_cursor_up(input, *cursor) {
                    *cursor = up;
                    return true;
                }
                match self.pos {
                    Some(p) => self.pos = Some(p.saturating_sub(1)),
                    None if !self.entries.is_empty() => {
                        self.draft = input.clone();
                        self.pos = Some(self.entries.len() - 1);
                    }
                    None => {}
                }
                if let Some(p) = self.pos {
                    *input = self.entries[p].clone();
                    *cursor = input.len();
                }
                true
            }
            KeyKind::Down => {
                if let Some(p) = self.pos {
                    if p + 1 < self.entries.len() {
                        self.pos = Some(p + 1);
                        *input = self.entries[p + 1].clone();
                    } else {
                        self.pos = None;
                        *input = std::mem::take(&mut self.draft);
                    }
                    *cursor = input.len();
                }
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn browse(h: &mut PromptHistory, input: &mut String, keys: &[KeyKind]) {
        let mut cursor = input.len();
        for k in keys {
            assert!(h.handle_key(k, input, &mut cursor));
        }
    }

    #[test]
    fn record_trims_and_skips_blank_and_repeats() {
        let mut h = PromptHistory::new(vec!["seeded".into()]);
        h.record("  first  ");
        h.record("first");
        h.record("   ");
        h.record("second");
        assert_eq!(h.entries(), ["seeded", "first", "second"]);
    }

    #[test]
    fn up_recalls_newest_first_and_down_restores_the_draft() {
        let mut h = PromptHistory::new(vec!["a".into(), "b".into()]);
        let mut input = "draft".to_string();
        browse(&mut h, &mut input, &[KeyKind::Up]);
        assert_eq!(input, "b");
        browse(&mut h, &mut input, &[KeyKind::Up, KeyKind::Up]);
        assert_eq!(input, "a", "stops at the oldest entry");
        browse(&mut h, &mut input, &[KeyKind::Down]);
        assert_eq!(input, "b");
        browse(&mut h, &mut input, &[KeyKind::Down]);
        assert_eq!(input, "draft");
    }

    #[test]
    fn a_prompt_recorded_mid_browse_is_recalled_next() {
        let mut h = PromptHistory::new(vec!["old".into()]);
        let mut input = String::new();
        browse(&mut h, &mut input, &[KeyKind::Up]);
        h.record("queued while busy");
        let mut input = String::new();
        browse(&mut h, &mut input, &[KeyKind::Up]);
        assert_eq!(input, "queued while busy");
    }

    #[test]
    fn other_keys_are_not_handled() {
        let mut h = PromptHistory::default();
        let (mut input, mut cursor) = (String::new(), 0);
        assert!(!h.handle_key(&KeyKind::Enter, &mut input, &mut cursor));
    }
}
