//! The rows a tool group puts in the transcript: an open group's calls (wrapped, a script's
//! calls nested under it), a folded group's failures and running calls, and the blank rows that
//! set a group apart from the prose around it. Split from `group_flow`, which owns membership,
//! the wrap and the keys.

use ratatui::text::Line as TextLine;

use super::group_flow::{Builder, FoldRow, FoldRowKind, GroupSpan, COMPACT_WIDTH, RUNNING_ROWS};
use super::group_rows::{self, CallRow};
use super::lines::rail;
use super::tool_cards::{self, CardStatus, ToolCard};
use super::tool_groups::{self, RowState, Summary};
use super::App;
use crate::copy_mark;

impl Builder<'_> {
    /// A row laid out to the width already: it is pushed as it is, never wrapped again.
    pub(super) fn exact(
        &mut self,
        line: TextLine<'static>,
        origin: usize,
        fold: (u64, FoldRowKind),
    ) {
        self.fold_rows.push(FoldRow {
            row: self.rows.len(),
            group: fold.0,
            kind: fold.1,
        });
        self.rows.push(line);
        self.origins.push(origin);
    }

    /// The rows of one call, marked so a click opens the card and the header can animate.
    pub(super) fn call(&mut self, r: &CallRow<'_>, group: u64, kind: impl Fn(bool) -> FoldRowKind) {
        for (i, row) in group_rows::call_rows(r).into_iter().enumerate() {
            self.exact(row, r.card.start, (group, kind(i == 0)));
        }
    }

    /// A bare rail row between a group and the prose next to it, so tool activity reads as a
    /// block of its own. Not after the turn's label, a blank row or anything outside a turn.
    pub(super) fn spacer(&mut self, origin: usize, group: u64, next: Option<&TextLine<'_>>) {
        let prev = self.rows.last();
        let in_turn = |l: &TextLine<'_>| l.spans.iter().take(2).any(copy_mark::is_rail);
        let wanted = match next {
            // Before the group: after a row of the turn's text.
            None => prev.is_some_and(|p| {
                in_turn(p) && !copy_mark::is_blank_row(p) && !rail::is_assistant_label(p)
            }),
            // After it: before a row of the turn's text.
            Some(n) => in_turn(n) && !copy_mark::is_blank_row(n),
        };
        if wanted {
            self.exact(
                TextLine::from(rail::rail_spans(rail::BAKED)),
                origin,
                (group, FoldRowKind::Spacer),
            );
        }
    }
}

impl App {
    /// A collapsed group: its summary (once a call has finished), its failures, and the calls
    /// still running. A call waiting for permission also shows its diff, because that is what
    /// the decision is about.
    pub(super) fn collapsed_rows(
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
        if summary.finished == 1 {
            // The one call is the summary; a failure's diagnosis still shows, under it.
            for card in cards.iter().filter(|c| c.status == CardStatus::Failed) {
                for note in tool_cards::note_rows(card) {
                    let kind = Some((span.id, FoldRowKind::Failed));
                    b.row(tool_groups::indented(note), card.start, kind);
                }
            }
        }
        if summary.finished > 1 {
            let failed: Vec<&&ToolCard> = cards
                .iter()
                .filter(|c| c.status == CardStatus::Failed)
                .collect();
            let skip = failed.len().saturating_sub(tool_groups::FAILED_ROWS);
            for card in failed.into_iter().skip(skip) {
                // A click here opens the group, not the card, so the card's own triangle goes.
                let row = CallRow {
                    card,
                    depth: 0,
                    children: &[],
                    width: b.width,
                    tick: None,
                    awaiting: false,
                    disclosure: false,
                };
                b.call(&row, span.id, |_| FoldRowKind::Failed);
                for note in tool_cards::note_rows(card) {
                    b.row(
                        tool_groups::indented(note),
                        card.start,
                        Some((span.id, FoldRowKind::Failed)),
                    );
                }
            }
        }
        // A script waiting on a call it made says nothing that call's own row does not.
        let busy_parent = |c: &&&ToolCard| {
            cards
                .iter()
                .any(|k| k.parent == Some(c.id) && k.status == CardStatus::Running)
        };
        for card in cards
            .iter()
            .filter(|c| c.status == CardStatus::Running && !busy_parent(c))
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

    /// An open group: each call wrapped to the width (`group_rows`), the calls a script made one
    /// level under it, and each card's own body (diff preview, failure tail, open drawer) under
    /// its header.
    pub(super) fn open_rows(
        &self,
        b: &mut Builder<'_>,
        span: &GroupSpan,
        cards: &[&ToolCard],
        awaiting: Option<u64>,
    ) {
        let mut at = span.start;
        for card in cards {
            for line in at..card.start {
                b.row(
                    tool_groups::indented(self.main_log[line].clone()),
                    line,
                    None,
                );
            }
            let nested = card.parent.is_some_and(|p| cards.iter().any(|c| c.id == p));
            let depth = usize::from(nested);
            let children = children_of(cards, card.id);
            let row = CallRow {
                card,
                depth,
                children: &children,
                width: b.width,
                tick: None,
                awaiting: awaiting == Some(card.id),
                disclosure: true,
            };
            b.call(&row, span.id, |first| FoldRowKind::Call {
                card: card.id,
                depth,
                first,
            });
            for line in card.start + 1..card.start + card.len {
                let mut body = self.main_log[line].clone();
                for _ in 0..=depth {
                    body = tool_groups::indented(body);
                }
                b.row(body, line, None);
            }
            at = card.start + card.len;
        }
        for line in at..span.end {
            b.row(
                tool_groups::indented(self.main_log[line].clone()),
                line,
                None,
            );
        }
    }
}

/// The calls a script card made, oldest first.
pub(super) fn children_of<'a>(cards: &[&'a ToolCard], id: u64) -> Vec<&'a ToolCard> {
    cards
        .iter()
        .filter(|c| c.parent == Some(id))
        .copied()
        .collect()
}
