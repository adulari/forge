//! UI state for folded tool groups: which groups are open, and which one has keyboard focus.

use std::collections::HashSet;

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
