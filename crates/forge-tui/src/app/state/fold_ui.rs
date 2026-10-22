//! UI state for folded tool groups: which groups are open, which one has keyboard focus, and
//! the summary each group last folded to.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::app::tool_cards::{CardStatus, ToolCard};
use crate::app::tool_groups::{self, Summary};

/// What a card contributes to its group's summary changes only with these: a call finishing, a
/// diff attaching, another read folding into the card.
type CardSig = (u64, CardStatus, usize, bool);

fn sig(card: &ToolCard) -> CardSig {
    (card.id, card.status, card.group.len(), card.diff.is_some())
}

/// A group's summary and the cards it was built from.
#[derive(Debug, Clone)]
struct Memo {
    cards: Vec<CardSig>,
    summary: Arc<Summary>,
}

/// What the transcript needs to know to fold tool groups. Expansion is keyed by group id (the id
/// of the group's first card), so it survives scrolling, resizing and the log growing.
#[derive(Debug, Clone, Default)]
pub(crate) struct FoldUi {
    /// Every group opens, including groups that arrive later (Alt+O).
    pub(crate) expand_all: bool,
    /// Groups toggled against `expand_all`.
    pub(crate) flipped: HashSet<u64>,
    /// The group the keyboard is on (Alt+↑/↓), if any.
    pub(crate) focus: Option<u64>,
    /// Bumped on every expand or collapse, so the wrapped transcript is rebuilt.
    pub(crate) rev: u64,
    /// Each group's summary, rebuilt only when one of its cards changes. The transcript rewraps
    /// on every streamed line, and a long session has hundreds of groups.
    summaries: RefCell<HashMap<u64, Memo>>,
}

impl FoldUi {
    pub(crate) fn expanded(&self, group: u64) -> bool {
        self.expand_all != self.flipped.contains(&group)
    }

    pub(crate) fn set(&mut self, group: u64, expanded: bool) {
        if expanded == self.expanded(group) {
            return;
        }
        if !self.flipped.remove(&group) {
            self.flipped.insert(group);
        }
        self.rev += 1;
    }

    /// The summary of `group`, built from `cards` unless they are unchanged since the last call.
    pub(crate) fn summary(&self, group: u64, cards: &[&ToolCard]) -> Arc<Summary> {
        let mut memo = self.summaries.borrow_mut();
        if let Some(m) = memo.get(&group) {
            if m.cards.len() == cards.len() && m.cards.iter().zip(cards).all(|(s, c)| *s == sig(c))
            {
                return Arc::clone(&m.summary);
            }
        }
        let summary = Arc::new(tool_groups::summarize(cards.iter().copied()));
        memo.insert(
            group,
            Memo {
                cards: cards.iter().map(|c| sig(c)).collect(),
                summary: Arc::clone(&summary),
            },
        );
        summary
    }

    /// Forget the summaries of groups no longer in the transcript.
    pub(crate) fn keep_summaries(&self, live: impl Fn(u64) -> bool) {
        self.summaries.borrow_mut().retain(|group, _| live(*group));
    }

    /// Forget every group: the transcript was cleared.
    pub(crate) fn clear(&mut self) {
        self.focus = None;
        self.flipped.clear();
        self.summaries.get_mut().clear();
        self.rev += 1;
    }

    /// Open every group, or fold every group when they are all open already.
    pub(crate) fn toggle_all(&mut self, any_collapsed: bool) {
        self.expand_all = any_collapsed;
        self.flipped.clear();
        self.rev += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_group_toggled_against_expand_all_keeps_its_own_state() {
        let mut ui = FoldUi::default();
        assert!(!ui.expanded(7));
        ui.set(7, true);
        assert!(ui.expanded(7) && !ui.expanded(8));
        ui.toggle_all(true);
        assert!(
            ui.expanded(7) && ui.expanded(8),
            "expand all opens every group"
        );
        ui.set(8, false);
        assert!(ui.expanded(7) && !ui.expanded(8));
        ui.toggle_all(false);
        assert!(!ui.expanded(7) && !ui.expanded(8) && !ui.expand_all);
    }

    #[test]
    fn setting_the_state_it_already_has_changes_nothing() {
        let mut ui = FoldUi::default();
        ui.set(3, false);
        assert_eq!(ui.rev, 0);
        ui.set(3, true);
        ui.set(3, true);
        assert_eq!(ui.rev, 1);
    }
}
