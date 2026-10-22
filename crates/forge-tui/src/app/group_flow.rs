//! Folding tool groups in the full-screen transcript.
//!
//! A group is a run of tool calls the assistant made without writing anything in between. A card
//! joins the open group when the group's last card is still the newest thing in the log as the
//! card starts; any other row (prose, a notice, a new turn) ends the group.
//!
//! `main_log` keeps every card exactly as it renders unfolded. Folding happens when the log is
//! wrapped: a collapsed group's lines are replaced by its summary row (plus its failures and its
//! running call), an open group gets the summary as a header above its cards. The wrap is
//! memoised on the log revision and the fold state, so a fold costs one pass over the log per
//! change, the same as the wrap it rides on. Each group's summary is memoised on its cards
//! (`FoldUi::summary`), so that pass rebuilds only the group whose cards changed, and a card's
//! file stat is counted once when its diff attaches. Only the rows that animate (a running
//! summary's spinner, the running call) or have focus are rebuilt at draw time.

use ratatui::text::Line as TextLine;
use std::collections::HashSet;

use super::tool_cards::{self, CardStatus, ToolCard};
use super::tool_groups::{self, RowState, Summary};
use super::{App, KeyKind, WrapCache};

/// Running calls a collapsed group shows rows for.
const RUNNING_ROWS: usize = 3;
/// Below this width a summary row drops its duration to keep more of its words.
const COMPACT_WIDTH: usize = 80;

/// Where a group sits in `main_log`, and the indices of its cards in `tool_cards`.
#[derive(Debug, Clone)]
pub(crate) struct GroupSpan {
    pub(crate) id: u64,
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) cards: Vec<usize>,
}

/// What a row the fold put on screen stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FoldRowKind {
    Summary,
    /// A running call; `indent` when it hangs under a summary row.
    Running {
        card: u64,
        indent: bool,
    },
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FoldRow {
    pub(crate) row: usize,
    pub(crate) group: u64,
    pub(crate) kind: FoldRowKind,
}

/// The rows a click toggles a group from: all of a collapsed group, the header of an open one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GroupHit {
    pub(crate) first: usize,
    pub(crate) rows: usize,
    pub(crate) group: u64,
}

/// Everything besides the log revision and width that changes how the log folds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct FoldKey {
    pub(crate) enabled: bool,
    pub(crate) rev: u64,
    pub(crate) awaiting: bool,
}

/// Rows being built for the wrap cache, with the fold annotations alongside.
struct Builder<'a> {
    width: usize,
    log: &'a [TextLine<'static>],
    rows: Vec<TextLine<'static>>,
    origins: Vec<usize>,
    fold_rows: Vec<FoldRow>,
    hits: Vec<GroupHit>,
}

impl Builder<'_> {
    fn log_lines(&mut self, from: usize, to: usize) {
        if from >= to {
            return;
        }
        let (rows, origins) =
            crate::transcript::wrap_lines_indexed(&self.log[from..to], self.width);
        self.rows.extend(rows);
        self.origins.extend(origins.into_iter().map(|i| i + from));
    }

    fn row(&mut self, line: TextLine<'static>, origin: usize, fold: Option<(u64, FoldRowKind)>) {
        for row in crate::transcript::wrap_lines(&[line], self.width) {
            if let Some((group, kind)) = fold {
                self.fold_rows.push(FoldRow {
                    row: self.rows.len(),
                    group,
                    kind,
                });
            }
            self.rows.push(row);
            self.origins.push(origin);
        }
    }
}

impl App {
    /// Whether tool groups fold: full-screen with `[tui] fold_tool_groups` on.
    pub(crate) fn fold_enabled(&self) -> bool {
        self.fullscreen && self.fold_tool_groups
    }

    pub(super) fn fold_key(&self) -> FoldKey {
        if !self.fold_enabled() {
            return FoldKey::default();
        }
        FoldKey {
            enabled: true,
            rev: self.fold_ui.rev,
            awaiting: self.prompt.is_some(),
        }
    }

    /// The group a card starting now belongs to: the open group when its last card is still the
    /// newest row of the log, else a new group named after this card.
    pub(super) fn group_for_new_card(&self, id: u64) -> u64 {
        match self.tool_cards.last() {
            Some(prev) if prev.fold.is_none() && self.card_is_last(prev) => {
                prev.batch.unwrap_or(id)
            }
            _ => id,
        }
    }

    /// The groups in the log, oldest first. Cards still queued in the flush have no place in the
    /// log yet and are left out until the next drain places them.
    pub(crate) fn group_spans(&self) -> Vec<GroupSpan> {
        let mut spans: Vec<GroupSpan> = Vec::new();
        for (i, card) in self.tool_cards.iter().enumerate() {
            let Some(batch) = card.batch else {
                continue;
            };
            if card.fold.is_some()
                || !card.interactive
                || self
                    .pending_card_marks
                    .iter()
                    .any(|(_, id, _)| *id == card.id)
                || card.start + card.len > self.main_log.len()
            {
                continue;
            }
            match spans.last_mut() {
                Some(span) if span.id == batch && card.start >= span.end => {
                    span.end = card.start + card.len;
                    span.cards.push(i);
                }
                _ => spans.push(GroupSpan {
                    id: batch,
                    start: card.start,
                    end: card.start + card.len,
                    cards: vec![i],
                }),
            }
        }
        spans
    }

    fn group_cards(&self, group: u64) -> impl Iterator<Item = &ToolCard> {
        self.tool_cards
            .iter()
            .filter(move |c| c.batch == Some(group) && c.fold.is_none())
    }

    pub(crate) fn group_expanded(&self, group: u64) -> bool {
        self.fold_ui.expanded(group)
    }

    /// The newest running call, which a permission prompt belongs to.
    fn newest_running(&self) -> Option<u64> {
        self.tool_cards
            .iter()
            .rev()
            .find(|c| c.status == CardStatus::Running && c.fold.is_none())
            .map(|c| c.id)
    }

    /// Wrap the log to `width` with every group folded the way the fold state says.
    pub(super) fn wrap_folded(&self, width: u16, cache: &mut WrapCache) {
        let mut b = Builder {
            width: width.saturating_sub(1) as usize,
            log: &self.main_log,
            rows: Vec::with_capacity(cache.rows.len().max(self.main_log.len())),
            origins: Vec::with_capacity(cache.origins.len().max(self.main_log.len())),
            fold_rows: Vec::new(),
            hits: Vec::new(),
        };
        let awaiting = self
            .prompt
            .is_some()
            .then(|| self.newest_running())
            .flatten();
        let spans = self.group_spans();
        // The newest group says how to open it while it is folded, the way Claude Code's
        // `(ctrl+o to expand)` does; older rows stay quiet.
        let hinted = spans
            .last()
            .filter(|s| !self.group_expanded(s.id))
            .map(|s| s.id);
        let mut at = 0;
        for span in &spans {
            if span.start < at {
                continue;
            }
            b.log_lines(at, span.start);
            at = span.end;
            let cards: Vec<&ToolCard> = span.cards.iter().map(|&i| &self.tool_cards[i]).collect();
            let summary = self.fold_ui.summary(span.id, &cards);
            let first = b.rows.len();
            let compact = b.width < COMPACT_WIDTH;
            if self.group_expanded(span.id) {
                let state = RowState {
                    expanded: true,
                    compact,
                    width: b.width,
                    ..RowState::default()
                };
                b.row(
                    tool_groups::summary_row(&summary, state),
                    span.start,
                    Some((span.id, FoldRowKind::Summary)),
                );
                b.hits.push(GroupHit {
                    first,
                    rows: 1,
                    group: span.id,
                });
                // The cards hang under the header, exactly as they render unfolded.
                for at in span.start..span.end {
                    b.row(tool_groups::indented(self.main_log[at].clone()), at, None);
                }
                continue;
            }
            let hint = (hinted == Some(span.id))
                .then(|| self.expand_hint())
                .flatten();
            self.collapsed_rows(&mut b, span, &cards, &summary, awaiting, hint);
            b.hits.push(GroupHit {
                first,
                rows: b.rows.len() - first,
                group: span.id,
            });
        }
        let live: HashSet<u64> = spans.iter().map(|s| s.id).collect();
        self.fold_ui.keep_summaries(|group| live.contains(&group));
        b.log_lines(at, self.main_log.len());
        cache.rows = b.rows;
        cache.origins = b.origins;
        cache.fold_rows = b.fold_rows;
        cache.group_hits = b.hits;
    }

    /// A collapsed group: its summary (once a call has finished), its failures, and the calls
    /// still running. A call waiting for permission also shows its diff, because that is what
    /// the decision is about.
    fn collapsed_rows(
        &self,
        b: &mut Builder<'_>,
        span: &GroupSpan,
        cards: &[&ToolCard],
        summary: &Summary,
        awaiting: Option<u64>,
        hint: Option<String>,
    ) {
        let has_summary = summary.finished > 0;
        if has_summary {
            let state = RowState {
                compact: b.width < COMPACT_WIDTH,
                width: b.width,
                // A group still running has nothing to open yet.
                hint: hint.filter(|_| summary.running == 0),
                ..RowState::default()
            };
            b.row(
                tool_groups::summary_row(summary, state),
                span.start,
                Some((span.id, FoldRowKind::Summary)),
            );
        }
        if summary.finished > 1 {
            let failed: Vec<&&ToolCard> = cards
                .iter()
                .filter(|c| c.status == CardStatus::Failed)
                .collect();
            let skip = failed.len().saturating_sub(tool_groups::FAILED_ROWS);
            for card in failed.into_iter().skip(skip) {
                // A click here opens the group, not the card, so the card's own triangle goes.
                let row = tool_groups::without_disclosure(tool_cards::live_header(card, 0, false));
                let row = tool_groups::indented(row);
                b.row(row, card.start, Some((span.id, FoldRowKind::Failed)));
            }
        }
        for card in cards
            .iter()
            .filter(|c| c.status == CardStatus::Running)
            .take(RUNNING_ROWS)
        {
            let waiting = awaiting == Some(card.id);
            let mut row = tool_cards::live_header(card, 0, waiting);
            if has_summary {
                row = tool_groups::indented(row);
            }
            let kind = FoldRowKind::Running {
                card: card.id,
                indent: has_summary,
            };
            b.row(row, card.start, Some((span.id, kind)));
            if waiting {
                for at in card.start + 1..card.start + card.len {
                    let mut body = self.main_log[at].clone();
                    if has_summary {
                        body = tool_groups::indented(body);
                    }
                    b.row(body, at, None);
                }
            }
        }
    }

    /// The fold row drawn at wrapped row `row`, if the fold put one there.
    pub(super) fn fold_row_at(cache: &WrapCache, row: usize) -> Option<FoldRow> {
        cache
            .fold_rows
            .binary_search_by_key(&row, |f| f.row)
            .ok()
            .map(|i| cache.fold_rows[i])
    }

    /// The row to draw over a fold row this frame, when it animates or has focus: a running
    /// group's spinner, the running call's header, the focused summary.
    pub(super) fn live_fold_row(&self, fold: FoldRow, width: u16) -> Option<TextLine<'static>> {
        let line = match fold.kind {
            FoldRowKind::Summary => {
                let focused = self.fold_ui.focus == Some(fold.group);
                let expanded = self.group_expanded(fold.group);
                let spinning = !expanded
                    && self
                        .group_cards(fold.group)
                        .any(|c| c.status == CardStatus::Running);
                if !focused && !spinning {
                    return None;
                }
                let cards: Vec<&ToolCard> = self.group_cards(fold.group).collect();
                let summary = self.fold_ui.summary(fold.group, &cards);
                let wrap = width.saturating_sub(1) as usize;
                let state = RowState {
                    expanded,
                    spinner: spinning.then_some(self.tick),
                    focused,
                    compact: wrap < COMPACT_WIDTH,
                    width: wrap,
                    hint: None,
                };
                tool_groups::summary_row(&summary, state)
            }
            FoldRowKind::Running { card, indent } => {
                let card = self.tool_cards.iter().find(|c| c.id == card)?;
                if card.status != CardStatus::Running {
                    return None;
                }
                let awaiting = self.prompt.is_some() && self.newest_running() == Some(card.id);
                let row = tool_cards::live_header(card, self.tick, awaiting);
                if indent {
                    tool_groups::indented(row)
                } else {
                    row
                }
            }
            FoldRowKind::Failed => return None,
        };
        crate::transcript::wrap_lines(&[line], width.saturating_sub(1) as usize)
            .into_iter()
            .next()
    }

    /// How a folded group opens, in the keys the user has bound: `click or Alt+O to expand`.
    fn expand_hint(&self) -> Option<String> {
        let chord = self
            .keybinds
            .binds
            .get("toggle_tool_groups")
            .map(crate::keybinds::combo_display);
        Some(match chord {
            Some(chord) => format!("click or {chord} to expand"),
            None => "click to expand".to_string(),
        })
    }

    /// The group whose clickable block holds wrapped row `row`.
    fn group_hit(&self, row: usize) -> Option<GroupHit> {
        let cache = self.wrap_cache.borrow();
        cache
            .group_hits
            .iter()
            .find(|h| row >= h.first && row < h.first + h.rows)
            .copied()
    }

    /// Whether a click on wrapped row `row` belongs to a group rather than to a card in it.
    pub(crate) fn row_belongs_to_group(&self, row: usize) -> bool {
        self.fold_enabled() && self.group_hit(row).is_some()
    }

    /// Toggle the group under a screen cell. True when a group was hit.
    pub(crate) fn toggle_group_at(&mut self, col: u16, row: u16) -> bool {
        if !self.fold_enabled() {
            return false;
        }
        let Some((wrapped_row, _)) = self.pointer_to_text(col, row) else {
            return false;
        };
        let width = self.transcript_geom.get().map_or(0, |g| g.width);
        self.ensure_wrapped_main(width);
        let Some(hit) = self.group_hit(wrapped_row) else {
            return false;
        };
        let open = self.group_expanded(hit.group);
        self.set_group_anchored(hit.group, !open);
        true
    }

    /// The wrapped row a group's summary sits on.
    fn group_row(&self, group: u64) -> Option<usize> {
        let cache = self.wrap_cache.borrow();
        cache
            .group_hits
            .iter()
            .find(|h| h.group == group)
            .map(|h| h.first)
    }

    /// Open or fold one group, keeping its summary on the same screen row so the content under
    /// the pointer does not jump.
    pub(crate) fn set_group_anchored(&mut self, group: u64, expanded: bool) {
        let Some(g) = self.transcript_geom.get() else {
            self.fold_ui.set(group, expanded);
            return;
        };
        self.ensure_wrapped_main(g.width);
        let before = self.group_row(group);
        self.fold_ui.set(group, expanded);
        self.ensure_wrapped_main(g.width);
        if let (Some(before), Some(after)) = (before, self.group_row(group)) {
            let screen = before.saturating_sub(g.scroll);
            self.scroll_to(after.saturating_sub(screen), g.width, g.height);
        }
    }

    /// Put wrapped row `top` at the top of the transcript; following resumes when that is the
    /// bottom.
    fn scroll_to(&mut self, top: usize, width: u16, height: u16) {
        let (_, max_scroll) = self.transcript_metrics(width, height);
        let top = top.min(max_scroll);
        self.transcript_follow = top >= max_scroll;
        self.transcript_scroll = if self.transcript_follow {
            usize::MAX / 2
        } else {
            top
        };
    }

    /// Alt+O: open every group, or fold them all when every group is open. The row at the top of
    /// the transcript stays where it is.
    pub fn toggle_all_tool_groups(&mut self) {
        let spans = self.group_spans();
        let any_collapsed = spans.iter().any(|s| !self.group_expanded(s.id));
        let anchor = self.top_anchor();
        self.fold_ui.toggle_all(any_collapsed);
        if let (Some((origin, offset)), Some(g)) = (anchor, self.transcript_geom.get()) {
            self.ensure_wrapped_main(g.width);
            let row = {
                let cache = self.wrap_cache.borrow();
                let at = cache.origins.partition_point(|&o| o < origin);
                match cache.origins.get(at) {
                    Some(&o) if o == origin => at + offset,
                    _ => at.saturating_sub(1),
                }
            };
            self.scroll_to(row, g.width, g.height);
        }
    }

    /// The log line at the top of the view and how far into its wrapped rows the view starts.
    /// `None` while following the tail, which keeps following.
    fn top_anchor(&self) -> Option<(usize, usize)> {
        if self.transcript_follow {
            return None;
        }
        let g = self.transcript_geom.get()?;
        self.ensure_wrapped_main(g.width);
        let cache = self.wrap_cache.borrow();
        let origin = *cache.origins.get(g.scroll)?;
        let first = cache.origins.partition_point(|&o| o < origin);
        Some((origin, g.scroll.saturating_sub(first)))
    }

    /// Move the keyboard focus to the next group down (or up), scrolling it into view. With no
    /// focus yet, start from the groups on screen.
    pub(crate) fn focus_tool_group(&mut self, down: bool) -> bool {
        let Some(g) = self.transcript_geom.get() else {
            return false;
        };
        self.ensure_wrapped_main(g.width);
        let heads: Vec<(usize, u64)> = {
            let cache = self.wrap_cache.borrow();
            cache
                .group_hits
                .iter()
                .map(|h| (h.first, h.group))
                .collect()
        };
        if heads.is_empty() {
            return false;
        }
        let view_end = g.scroll + g.height as usize;
        let current = self
            .fold_ui
            .focus
            .and_then(|id| heads.iter().position(|(_, g)| *g == id));
        let next = match (current, down) {
            (Some(i), true) => (i + 1).min(heads.len() - 1),
            (Some(i), false) => i.saturating_sub(1),
            (None, true) => heads
                .iter()
                .position(|(row, _)| *row >= g.scroll)
                .unwrap_or(heads.len() - 1),
            (None, false) => heads
                .iter()
                .rposition(|(row, _)| *row < view_end)
                .unwrap_or(0),
        };
        let (row, id) = heads[next];
        self.fold_ui.focus = Some(id);
        if row < g.scroll || row >= view_end {
            let top = row.saturating_sub(g.height as usize / 3);
            self.scroll_to(top, g.width, g.height);
        }
        true
    }

    /// Keys for tool groups. Alt+↑/↓ move the focus between groups and Alt+O opens or folds
    /// them all; while a group has focus, ↑/↓ move it, Enter or Space toggle it and Esc lets go.
    /// Any other key lets go and is handled as usual. True when the key was consumed.
    pub fn tool_group_key(&mut self, key: KeyKind) -> bool {
        let explicit = matches!(
            key,
            KeyKind::PrevToolGroup | KeyKind::NextToolGroup | KeyKind::ToggleToolGroups
        );
        if explicit && !self.fold_enabled() {
            self.note(if self.fullscreen {
                "tool groups are not folded: [tui] fold_tool_groups is off"
            } else {
                "tool groups fold in the full-screen transcript only"
            });
            return true;
        }
        match key {
            KeyKind::PrevToolGroup | KeyKind::NextToolGroup => {
                if !self.focus_tool_group(key == KeyKind::NextToolGroup) {
                    self.note("no tool calls to move between yet");
                }
                return true;
            }
            KeyKind::ToggleToolGroups => {
                self.toggle_all_tool_groups();
                return true;
            }
            _ => {}
        }
        let Some(group) = self.fold_ui.focus else {
            return false;
        };
        // A decision (permission, question) owns the keys while it waits.
        if self.prompt.is_some() || self.form.is_some() {
            self.fold_ui.focus = None;
            return false;
        }
        match key {
            KeyKind::Up | KeyKind::Down => {
                self.focus_tool_group(key == KeyKind::Down);
            }
            KeyKind::Enter | KeyKind::Char(' ') => {
                let open = self.group_expanded(group);
                self.set_group_anchored(group, !open);
            }
            KeyKind::Esc => self.fold_ui.focus = None,
            KeyKind::PageUp | KeyKind::PageDown | KeyKind::JumpBottom | KeyKind::JumpPrevUser => {
                return false;
            }
            _ => {
                self.fold_ui.focus = None;
                return false;
            }
        }
        true
    }

    /// Ctrl+T on a card that sits in a group: a folded group opens, an open group opens its
    /// newest card, and an open card folds the whole group again. False when the newest card is
    /// not in a group, so the caller toggles the card as before.
    pub(super) fn cycle_group_of(&mut self, card_id: u64) -> bool {
        if !self.fold_enabled() {
            return false;
        }
        let Some(card) = self.tool_cards.iter().find(|c| c.id == card_id) else {
            return false;
        };
        let Some(group) = card.batch.filter(|_| card.fold.is_none()) else {
            return false;
        };
        let (has_detail, card_open) = (card.has_detail(), card.expanded);
        if !self.group_expanded(group) {
            self.set_group_anchored(group, true);
        } else if has_detail && !card_open {
            if let Some(card) = self.tool_cards.iter_mut().find(|c| c.id == card_id) {
                card.expanded = true;
            }
            self.redraw_tool_card(card_id);
        } else {
            if card_open {
                if let Some(card) = self.tool_cards.iter_mut().find(|c| c.id == card_id) {
                    card.expanded = false;
                }
                self.redraw_tool_card(card_id);
            }
            self.set_group_anchored(group, false);
        }
        true
    }
}

#[cfg(test)]
#[path = "group_flow_tests.rs"]
mod tests;
