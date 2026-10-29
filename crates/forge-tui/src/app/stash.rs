//! The prompt stash: park a half-written draft, ask something else, bring the draft back.

use super::*;
use crate::stash_args::StashOp;

/// Drafts kept; the oldest falls off.
const STASH_MAX: usize = 20;

impl App {
    /// The stash key: with a draft in the composer, park it; with an empty composer, bring back
    /// the newest parked draft.
    pub fn stash_toggle(&mut self) {
        if self.input.trim().is_empty() {
            self.stash_pop();
        } else {
            self.stash_push();
        }
    }

    pub fn stash_push(&mut self) {
        if self.input.trim().is_empty() {
            self.note("nothing to stash — the composer is empty");
            return;
        }
        let draft = std::mem::take(&mut self.input);
        self.input_cursor = 0;
        self.input_edit = InputEditState::default();
        self.stash.push(draft);
        if self.stash.len() > STASH_MAX {
            self.stash.remove(0);
        }
        self.note(&format!(
            "⇣ draft stashed ({} parked) — empty composer + stash key brings it back",
            self.stash.len()
        ));
    }

    pub fn stash_pop(&mut self) {
        let Some(draft) = self.stash.pop() else {
            self.note("the stash is empty");
            return;
        };
        if !self.input.trim().is_empty() {
            self.stash.push(draft);
            self.note("the composer has a draft — stash it first, or clear it");
            return;
        }
        self.input_cursor = draft.len();
        self.input = draft;
        self.input_edit = InputEditState::default();
    }

    pub fn stash_command(&mut self, op: StashOp) {
        match op {
            StashOp::Push => self.stash_push(),
            StashOp::Pop => self.stash_pop(),
            StashOp::Clear => {
                let n = std::mem::take(&mut self.stash).len();
                self.note(&format!("stash cleared ({n} dropped)"));
            }
            StashOp::List => {
                if self.stash.is_empty() {
                    self.note("the stash is empty");
                    return;
                }
                let mut lines = vec![format!("stash · {} parked, newest first", self.stash.len())];
                for (i, draft) in self.stash.iter().rev().enumerate() {
                    let flat = draft.split_whitespace().collect::<Vec<_>>().join(" ");
                    let shown: String = flat.chars().take(80).collect();
                    lines.push(format!("  {}. {shown}", i + 1));
                }
                self.note(&lines.join("\n"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_with(text: &str) -> App {
        App {
            input: text.to_string(),
            input_cursor: text.len(),
            ..App::default()
        }
    }

    #[test]
    fn the_stash_key_parks_a_draft_then_restores_it() {
        let mut app = app_with("half a thought");
        app.edit_key(KeyKind::StashPrompt);
        assert!(app.input.is_empty());
        assert_eq!(app.stash, ["half a thought"]);
        app.edit_key(KeyKind::StashPrompt);
        assert_eq!(app.input, "half a thought");
        assert_eq!(app.input_cursor, app.input.len());
        assert!(app.stash.is_empty());
    }

    #[test]
    fn drafts_come_back_newest_first() {
        let mut app = app_with("one");
        app.stash_push();
        app.input = "two".into();
        app.stash_push();
        app.stash_pop();
        assert_eq!(app.input, "two");
    }

    #[test]
    fn a_pop_never_overwrites_a_draft() {
        let mut app = app_with("parked");
        app.stash_push();
        app.input = "typing".into();
        app.stash_pop();
        assert_eq!(app.input, "typing");
        assert_eq!(app.stash, ["parked"]);
    }

    #[test]
    fn the_stash_is_bounded() {
        let mut app = App::default();
        for i in 0..30 {
            app.input = format!("d{i}");
            app.stash_push();
        }
        assert_eq!(app.stash.len(), STASH_MAX);
        assert_eq!(app.stash[0], "d10");
    }
}
