use crate::{StatuslineConfig, StatuslineWidget};

/// What [`StatuslineConfig::toggle_widget`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WidgetToggle {
    RemovedRight,
    RemovedLeft,
    AddedRight,
}

impl StatuslineConfig {
    /// `/statusline toggle <widget>`: remove it from the right or left segment, else add it to
    /// the right. Toggling `anywhere_sync` also moves the auto badge's switch, so a listed
    /// widget that is removed does not come straight back as the auto badge.
    pub fn toggle_widget(&mut self, widget: StatuslineWidget) -> WidgetToggle {
        let is_sync = widget == StatuslineWidget::AnywhereSync;
        let result = if let Some(i) = self.right.iter().position(|w| w == &widget) {
            self.right.remove(i);
            WidgetToggle::RemovedRight
        } else if let Some(i) = self.left.iter().position(|w| w == &widget) {
            self.left.remove(i);
            WidgetToggle::RemovedLeft
        } else {
            self.right.push(widget);
            WidgetToggle::AddedRight
        };
        if is_sync {
            self.hide_sync_badge = result != WidgetToggle::AddedRight;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggling_a_listed_sync_widget_off_does_not_bring_the_auto_badge_back() {
        let mut cfg = StatuslineConfig {
            right: vec![StatuslineWidget::AnywhereSync],
            ..StatuslineConfig::default()
        };
        assert_eq!(
            cfg.toggle_widget(StatuslineWidget::AnywhereSync),
            WidgetToggle::RemovedRight
        );
        assert!(
            !cfg.auto_sync_badge(),
            "the removed widget must stay hidden"
        );
        assert_eq!(
            cfg.toggle_widget(StatuslineWidget::AnywhereSync),
            WidgetToggle::AddedRight
        );
        assert!(cfg.lists(&StatuslineWidget::AnywhereSync));
        assert!(!cfg.hide_sync_badge);
    }
}
