//! The turn state machine of the transcript: one assistant label per turn, answer segments,
//! the faint footer, notices that collapse when they repeat, and the rail that cools when a
//! turn settles.
//!
//! The builders in `lines/` are pure; everything here decides WHEN a row is written and what
//! state carries between events.

use std::time::Instant;

use ratatui::style::Style;
use ratatui::text::{Line as TextLine, Span};

use super::lines::{notice, rail, turn};
use super::state::transcript_ui::{Cooling, NoticeRun, COOL_STEP_MS};
use super::{human, App, LineOrigin};
use crate::copy_mark;
use crate::theme::glyphs::g;
use crate::theme::styles::{self, NoticeKind, RailKind};

/// Rows an answer must exceed (or tools it must have used) for a recap to be worth showing.
const RECAP_MIN_ROWS: usize = 12;
const RECAP_MIN_TOOLS: usize = 3;

impl App {
    /// Write the assistant's label row, once per turn: identity on the left, the routed model
    /// and tier right-aligned.
    pub(super) fn ensure_assistant_label(&mut self) {
        if self.transcript_ui.turn_open {
            return;
        }
        self.flush_notice_run();
        let (model, tier) = match &self.routing {
            Some(r) => (Some(super::model_short(Some(&r.model))), r.tier.clone()),
            None => (None, String::new()),
        };
        if self.fullscreen {
            self.transcript_ui.cooling = Some(Cooling {
                from: self.main_log.len() + self.flush.len(),
                since: None,
                stage: 0,
            });
        }
        self.transcript_ui.turn_open = true;
        self.transcript_ui
            .turn_started
            .get_or_insert_with(Instant::now);
        // A blank row sets the turn apart from whatever came before it (a notice, say).
        let blank_before = match self.flush.last().or_else(|| self.main_log.last()) {
            Some(prev) => prev.spans.iter().all(|s| s.content.trim().is_empty()),
            None => true,
        };
        let label = turn::assistant_label(model.as_deref(), &tier);
        self.tag_flush(LineOrigin::assistant(), |s| {
            if !blank_before {
                s.flush.push(TextLine::default());
            }
            s.flush.push(label);
        });
    }

    /// One answer segment of the turn. A segment identical to the one before it (a re-answer
    /// after the completeness review) is dropped rather than appended.
    pub(super) fn push_answer_segment(&mut self, text: &str, rows: Vec<TextLine<'static>>) {
        let norm = text.trim().to_string();
        if norm.is_empty() || rows.is_empty() {
            return;
        }
        if self.transcript_ui.segments.last() == Some(&norm) {
            return;
        }
        self.transcript_ui.segments.push(norm);
        self.push_turn_lines(rows);
    }

    /// An error notice, with a `Next:` line when a fix is known.
    pub(super) fn push_error(&mut self, text: &str) {
        if self.streaming_active && !self.streaming.is_empty() {
            self.finish_streaming_text();
        }
        self.flush_notice_run();
        let in_turn = self.transcript_ui.turn_open;
        let mut rows = notice::notice_lines(NoticeKind::Err, in_turn, text);
        if let Some(fix) = notice::fix_hint(text) {
            rows.push(notice::next_step(fix, in_turn));
        }
        self.seal_flush_origins(&LineOrigin::default());
        self.flush.extend(rows);
    }

    /// Rows of the assistant's turn: the rail goes in front of each.
    pub(super) fn push_turn_lines(&mut self, lines: Vec<TextLine<'static>>) {
        let rows = rail::rail_lines(lines, rail::BAKED);
        self.tag_flush(LineOrigin::assistant(), |s| s.flush.extend(rows));
    }

    /// Start a user turn: whatever turn was open ends here, and the per-turn state resets.
    pub(super) fn begin_user_turn(&mut self) {
        self.flush_notice_run();
        self.settle_running_cards();
        if self.transcript_ui.turn_open {
            self.flush.push(TextLine::default());
        }
        let ui = &mut self.transcript_ui;
        ui.turn_open = false;
        ui.cooling = None;
        ui.segments.clear();
        ui.tools_used = 0;
        ui.reasoning_since = None;
        ui.turn_started = Some(Instant::now());
        ui.cost_base = self.cost_usd;
    }

    /// End the open turn: its footer row, then the blank row that separates turns. The rail
    /// starts to cool.
    pub(super) fn close_turn(&mut self) {
        self.flush_notice_run();
        if !self.transcript_ui.turn_open {
            return;
        }
        let facts = self.turn_facts();
        self.transcript_ui.turn_open = false;
        let footer = (!facts.is_empty()).then(|| turn::turn_footer(&facts));
        self.tag_flush(LineOrigin::default(), |s| {
            s.flush.extend(footer);
            s.flush.push(TextLine::default());
        });
        if let Some(c) = &mut self.transcript_ui.cooling {
            if crate::theme::t().animates() {
                c.since = Some(Instant::now());
                c.stage = 0;
            } else {
                self.transcript_ui.cooling = None;
            }
        }
    }

    /// `0.9s · ↑72 ↓30 · free`: what the turn that just ended cost.
    fn turn_facts(&self) -> String {
        let glyphs = g();
        let mut parts: Vec<String> = Vec::new();
        if let Some(t) = self.transcript_ui.turn_started {
            let secs = t.elapsed().as_secs_f64();
            parts.push(if secs < 10.0 {
                format!("{secs:.1}s")
            } else {
                super::render::fmt_dur(secs as u64)
            });
        }
        if self.turn_in + self.turn_out > 0 {
            parts.push(format!(
                "{}{} {}{}",
                glyphs.tok_up,
                human(self.turn_in),
                glyphs.tok_down,
                human(self.turn_out)
            ));
        }
        if let Some(r) = &self.routing {
            let delta = (self.cost_usd - self.transcript_ui.cost_base).max(0.0);
            let cost = super::render::cost_cell(&r.model, delta);
            if cost != "untracked" {
                parts.push(cost);
            }
        }
        parts.join(&format!(" {} ", glyphs.sep))
    }

    /// Show one notice. Inside an open turn it rides the rail; a notice whose text repeats with
    /// only its digits changed folds into one `×N` row.
    pub(super) fn push_notice(&mut self, kind: NoticeKind, text: &str) {
        if self.streaming_active && !self.streaming.is_empty() {
            // The answer streamed so far is a segment of its own; what follows is new.
            self.finish_streaming_text();
        }
        let mut text = text.trim_end().to_string();
        if text.starts_with("CLI-bridge runs") || text.starts_with("CLI bridge:") {
            if std::mem::replace(&mut self.transcript_ui.bridge_notice_seen, true) {
                return;
            }
            text = "CLI bridge: runs your local claude/codex login; provider terms may limit \
                    third-party use, see docs/features/provider-integrations.md"
                .to_string();
        }
        let in_turn = self.transcript_ui.turn_open;
        let key = notice::repeat_key(&text);
        let total = self.main_log.len() + self.flush.len();
        let single_row = !text.contains('\n');
        if let Some(run) = &mut self.transcript_ui.notice_run {
            if single_row
                && run.key == key
                && run.kind == kind
                && run.in_turn == in_turn
                && (run.end == total || run.held)
            {
                run.count += 1;
                run.text = text;
                if run.held || !self.fullscreen {
                    run.held = true;
                    return;
                }
                let (start, end) = (run.start, run.end);
                let rows = notice::counted(kind, in_turn, &run.text, run.count);
                self.rewrite_rows(start, end, rows);
                return;
            }
        }
        self.flush_notice_run();
        let rows = notice::notice_lines(kind, in_turn, &text);
        let start = self.main_log.len() + self.flush.len();
        let n = rows.len();
        self.seal_flush_origins(&LineOrigin::default());
        self.flush.extend(rows);
        self.transcript_ui.notice_run = single_row.then_some(NoticeRun {
            key,
            kind,
            text,
            count: 1,
            in_turn,
            start,
            end: start + n,
            held: false,
        });
    }

    /// Print the held `×N` row of a notice that repeated inline, and forget the run.
    pub(super) fn flush_notice_run(&mut self) {
        let Some(run) = self.transcript_ui.notice_run.take() else {
            return;
        };
        if run.held && run.count > 1 {
            let rows = notice::counted(run.kind, run.in_turn, &run.text, run.count);
            self.seal_flush_origins(&LineOrigin::default());
            self.flush.extend(rows);
        }
    }

    /// Replace the rows `start..end` of the log (queued or already drained) in place.
    fn rewrite_rows(&mut self, start: usize, end: usize, rows: Vec<TextLine<'static>>) {
        let drained = self.main_log.len();
        if rows.len() != end - start {
            return;
        }
        for (i, row) in rows.into_iter().enumerate() {
            let at = start + i;
            if at >= drained {
                if let Some(slot) = self.flush.get_mut(at - drained) {
                    *slot = row;
                }
            } else if let Some(slot) = self.main_log.get_mut(at) {
                *slot = row;
                self.main_log_rev += 1;
            }
        }
    }

    /// Whether the recap under an answer adds anything: only a long or tool-heavy turn needs one,
    /// and never one that merely restates the answer.
    pub(super) fn recap_is_worth_showing(&self, recap: &str) -> bool {
        let answer: String = self.transcript_ui.segments.join("\n");
        let rows = answer.lines().count();
        if rows <= RECAP_MIN_ROWS && self.transcript_ui.tools_used < RECAP_MIN_TOOLS {
            return false;
        }
        let norm = |s: &str| {
            s.split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase()
        };
        let (recap, answer) = (norm(recap), norm(&answer));
        let head: String = answer.chars().take(40).collect();
        !(recap.is_empty() || answer.contains(&recap) || recap.starts_with(&head))
    }

    pub(super) fn recap_row(&self, text: &str) -> TextLine<'static> {
        let mut spans = rail::rail_spans(rail::BAKED);
        spans.push(Span::styled(
            String::new(),
            copy_mark::prose(Style::default()),
        ));
        spans.push(Span::styled("recap  ", styles::muted().bold()));
        spans.push(Span::styled(text.to_string(), styles::muted()));
        TextLine::from(spans)
    }

    /// Advance the rail's cooling. True while the screen needs another draw for it.
    pub fn tick_cooling(&mut self) -> bool {
        let Some(c) = &mut self.transcript_ui.cooling else {
            return false;
        };
        let Some(since) = c.since else {
            return false;
        };
        let steps = (since.elapsed().as_millis() / COOL_STEP_MS) as u8;
        if steps >= 2 {
            self.transcript_ui.cooling = None;
            return true;
        }
        if steps != c.stage {
            c.stage = steps;
            return true;
        }
        false
    }

    /// The style the open turn's rail is drawn in, and the first log line it applies from.
    /// `None` once the rail has finished cooling (the baked colour is the settled one).
    pub(crate) fn hot_rail(&self) -> Option<(usize, Style)> {
        let c = self.transcript_ui.cooling.as_ref()?;
        let kind = match c.since {
            None => RailKind::Live,
            Some(_) => RailKind::Settled(c.stage),
        };
        Some((c.from, styles::rail(kind)))
    }
}

/// Re-style the assistant rail cell of a row (a cooling turn is redrawn each tick).
pub(crate) fn restyle_rail(row: &mut TextLine<'static>, style: Style) {
    let glyph = g().rail_assistant;
    if let Some(cell) = row
        .spans
        .iter_mut()
        .take(3)
        .find(|s| s.content.as_ref() == glyph)
    {
        cell.style = copy_mark::repeat(style);
    }
}
