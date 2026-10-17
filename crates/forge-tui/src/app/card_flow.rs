//! Tool cards in the transcript: opening one at `ToolStart`, attaching the diff, closing it on
//! the result, redrawing it in place, and the folded user turn that shares the machinery.
//!
//! Full-screen cards live in `main_log` and are rewritten where they sit. Inline cards are held
//! back until the result lands and printed once.

use ratatui::text::Line as TextLine;

use super::tool_cards::{self, CardStatus, ToolCard, UserFold, USER_FOLD_ROWS};
use super::{sanitize_terminal_text, App, LineOrigin};

/// Canned command prompts, and what the transcript calls them instead of echoing 500 characters.
const CANNED: &[(&str, &str)] = &[
    (
        "Set up Forge for this repository",
        "Set up Forge for this project",
    ),
    (
        "Investigate the codebase as needed, then produce a concrete",
        "Plan a change",
    ),
];

impl App {
    /// The cell width a card renders to. Two columns come off the area width: the transcript
    /// itself reserves one when it wraps (`ensure_wrapped_main`), and `wrap_lines` breaks AT the
    /// wrap width, so a row filling it exactly still emits a second, empty row. A card row must
    /// stay one wrapped row — a card that occupies two rows shows a blank gap under every call
    /// and desynchronises click hit-testing from the card's own line range.
    pub(super) fn card_width(&self) -> u16 {
        self.last_width.get().saturating_sub(2)
    }

    /// Where a card's rows end, for the "is it the newest thing in the log" test: the log
    /// position just past them.
    fn card_is_last(&self, card: &ToolCard) -> bool {
        match self
            .pending_card_marks
            .iter()
            .find(|(_, id, _)| *id == card.id)
        {
            Some((offset, _, len)) => offset + len == self.flush.len(),
            None => self.flush.is_empty() && card.start + card.len == self.main_log.len(),
        }
    }

    /// A finished read the next read can fold into: the newest row of the log, collapsed.
    fn foldable_read(&self) -> Option<u64> {
        let card = self.tool_cards.last()?;
        (card.fold.is_none()
            && card.name == "read_file"
            && card.status == CardStatus::Ok
            && !card.expanded
            && card.interactive
            && self.card_is_last(card))
        .then_some(card.id)
    }

    /// Start a tool card. Full-screen queues its row into the flush like any other scrollback
    /// line and remembers where in this flush it sits so `drain_flush` can record its final
    /// position in `main_log` (the only point at which that position is known). Inline holds the
    /// card back: the running call shows in the composer border, and the finished row is
    /// printed once when the result lands.
    pub(super) fn open_tool_card(&mut self, origin: LineOrigin, name: String, args: String) {
        if self.fullscreen && name == "read_file" {
            if let Some(id) = self.foldable_read() {
                if let Some(card) = self.tool_cards.iter_mut().find(|c| c.id == id) {
                    card.fold_in(name, args);
                }
                self.redraw_tool_card(id);
                return;
            }
        }
        let id = self.next_card_id;
        self.next_card_id += 1;
        let mut card = ToolCard::new(id, name, args);
        card.interactive = self.fullscreen;
        if !self.fullscreen {
            self.tool_cards.push(card);
            return;
        }
        let lines = tool_cards::card_lines(&card, self.card_width());
        let len = lines.len();
        self.tool_cards.push(card);
        self.seal_flush_origins(&LineOrigin::default());
        self.pending_card_marks.push((self.flush.len(), id, len));
        self.tag_flush(origin, |s| s.flush.extend(lines));
    }

    /// Give the newest running call the file change it is about to make. False when no call is
    /// waiting for one.
    pub(super) fn attach_diff_to_card(&mut self, diff: forge_types::FileDiff) -> bool {
        let Some(card) = self
            .tool_cards
            .iter_mut()
            .rev()
            .find(|c| c.fold.is_none() && c.status == CardStatus::Running && c.diff.is_none())
        else {
            return false;
        };
        card.diff = Some(diff);
        let (id, interactive) = (card.id, card.interactive);
        if interactive {
            self.redraw_tool_card(id);
        }
        true
    }

    /// Finish the newest still-running card for `name`. Returns false when there is no such
    /// card (a result with no matching start — e.g. a call refused before its `ToolStart` was
    /// emitted), so the caller can fall back to a result-only card.
    pub(super) fn close_tool_card(
        &mut self,
        name: &str,
        ok: bool,
        summary: &str,
        detail: Option<String>,
    ) -> bool {
        let Some(card) = self
            .tool_cards
            .iter_mut()
            .rev()
            .find(|c| c.fold.is_none() && c.name == name && c.status == CardStatus::Running)
        else {
            return false;
        };
        card.status = if ok {
            CardStatus::Ok
        } else {
            CardStatus::Failed
        };
        card.ms = Some(card.started.elapsed().as_millis());
        card.summary = sanitize_terminal_text(summary);
        card.detail = detail.map(|d| sanitize_terminal_text(&d));
        let id = card.id;
        self.last_closed_card = Some(id);
        if !self.fullscreen {
            self.print_inline_card(id, name, ok);
            return true;
        }
        self.redraw_tool_card(id);
        // The redraw rewrote `main_log` directly; the remote transcript ring only ever sees
        // flushed lines, so hand it the finished row explicitly or the phone would be left
        // showing this call as still running. Not needed when the card was still queued — the
        // redraw rewrote the queued row itself, which the ring is about to receive anyway, and
        // pushing here too would show the phone the same call twice.
        let still_queued = self.pending_card_marks.iter().any(|(_, mid, _)| *mid == id);
        if let Some(card) = self
            .tool_cards
            .iter()
            .find(|c| c.id == id)
            .filter(|_| !still_queued)
        {
            let row = tool_cards::card_lines(card, self.card_width())
                .into_iter()
                .next();
            if let Some(row) = row {
                self.remote_only
                    .push((row, LineOrigin::tool_result(name, ok)));
            }
        }
        true
    }

    /// A turn ended with calls still running (it was interrupted mid-tool): mark them so a card
    /// never spins for the rest of the session. Inline cards that never printed are dropped.
    pub(super) fn settle_running_cards(&mut self) {
        let running: Vec<u64> = self
            .tool_cards
            .iter()
            .filter(|c| c.fold.is_none() && c.status == CardStatus::Running)
            .map(|c| c.id)
            .collect();
        for id in running {
            let Some(card) = self.tool_cards.iter_mut().find(|c| c.id == id) else {
                continue;
            };
            if !card.interactive {
                self.tool_cards.retain(|c| c.id != id);
                continue;
            }
            card.status = CardStatus::Failed;
            card.ms = Some(card.started.elapsed().as_millis());
            card.summary = "interrupted".to_string();
            self.redraw_tool_card(id);
        }
    }

    /// Print a finished inline card, once, and let go of it (its row can no longer change).
    fn print_inline_card(&mut self, id: u64, name: &str, ok: bool) {
        let Some(pos) = self.tool_cards.iter().position(|c| c.id == id) else {
            return;
        };
        let card = self.tool_cards.remove(pos);
        let lines = tool_cards::card_lines(&card, self.card_width());
        self.tag_flush(LineOrigin::tool_result(name, ok), |s| s.flush.extend(lines));
    }

    /// A result with no call before it: print it as a card of its own.
    pub(super) fn print_orphan_result(
        &mut self,
        name: &str,
        ok: bool,
        summary: &str,
        detail: Option<String>,
    ) {
        let id = self.next_card_id;
        self.next_card_id += 1;
        let mut card = ToolCard::new(id, name.to_string(), String::new());
        card.interactive = false;
        card.status = if ok {
            CardStatus::Ok
        } else {
            CardStatus::Failed
        };
        card.summary = sanitize_terminal_text(summary);
        card.detail = detail.map(|d| sanitize_terminal_text(&d));
        let lines = tool_cards::card_lines(&card, self.card_width());
        self.tag_flush(LineOrigin::tool_result(name, ok), |s| s.flush.extend(lines));
    }

    /// Re-render one card over the lines it currently occupies, shifting the cards after it by the
    /// change in height. Bumps the log revision so the wrap cache re-wraps.
    pub(super) fn redraw_tool_card(&mut self, id: u64) {
        let width = self.card_width();
        let Some(pos) = self.tool_cards.iter().position(|c| c.id == id) else {
            return;
        };
        // A card still carrying a pending mark has no place in `main_log` yet — its lines are
        // sitting in the flush, unclaimed. That is the common case for a fast tool, which starts
        // and finishes inside a single frame: rewrite the QUEUED lines. Splicing `main_log` at the
        // card's still-unset position would instead insert a second copy of the row at the very
        // top of the transcript, above the banner, while the queued row stayed "running" forever.
        if let Some(mark) = self
            .pending_card_marks
            .iter()
            .position(|(_, mid, _)| *mid == id)
        {
            let (offset, _, old_len) = self.pending_card_marks[mark];
            if offset + old_len > self.flush.len() {
                return;
            }
            let lines = tool_cards::card_lines(&self.tool_cards[pos], width);
            let new_len = lines.len();
            self.flush.splice(offset..offset + old_len, lines);
            // `flush_origins` runs in lockstep with `flush`; keep the replaced lines' provenance
            // (a tool call) rather than letting a later seal reclassify them.
            if self.flush_origins.len() >= offset + old_len {
                let origin = self.flush_origins[offset].clone();
                self.flush_origins.splice(
                    offset..offset + old_len,
                    std::iter::repeat_n(origin, new_len),
                );
            }
            self.tool_cards[pos].len = new_len;
            self.pending_card_marks[mark].2 = new_len;
            // Every later mark in this same flush shifts by the change in height.
            let delta = new_len as isize - old_len as isize;
            if delta != 0 {
                for (other_offset, _, _) in self.pending_card_marks.iter_mut().skip(mark + 1) {
                    *other_offset = (*other_offset as isize + delta).max(0) as usize;
                }
            }
            return;
        }
        let (start, old_len) = (self.tool_cards[pos].start, self.tool_cards[pos].len);
        if start + old_len > self.main_log.len() {
            return; // its region no longer exists (log cleared under it)
        }
        let lines = tool_cards::card_lines(&self.tool_cards[pos], width);
        let new_len = lines.len();
        self.main_log.splice(start..start + old_len, lines);
        self.tool_cards[pos].len = new_len;
        let delta = new_len as isize - old_len as isize;
        if delta != 0 {
            for card in self.tool_cards.iter_mut().skip(pos + 1) {
                card.start = (card.start as isize + delta).max(0) as usize;
            }
            // Positions the transcript tracks past this card move with it.
            if let Some(c) = &mut self.transcript_ui.cooling {
                if c.from > start {
                    c.from = (c.from as isize + delta).max(0) as usize;
                }
            }
            if let Some(run) = &mut self.transcript_ui.notice_run {
                if run.start > start {
                    run.start = (run.start as isize + delta).max(0) as usize;
                    run.end = (run.end as isize + delta).max(0) as usize;
                }
            }
        }
        self.main_log_rev += 1;
        if self.transcript_follow {
            self.transcript_scroll = usize::MAX / 2;
        }
    }

    /// The headers of the cards still running, as the draw loop shows them: an animated
    /// spinner, or `◇ awaiting approval` on the newest while a permission prompt is open. Each is
    /// paired with the log line it replaces.
    pub(crate) fn running_card_headers(&self, width: usize) -> Vec<(usize, TextLine<'static>)> {
        let newest = self
            .tool_cards
            .iter()
            .rev()
            .find(|c| c.status == CardStatus::Running && c.fold.is_none())
            .map(|c| c.id);
        self.tool_cards
            .iter()
            .filter(|c| c.fold.is_none() && c.interactive && c.status == CardStatus::Running)
            .filter(|c| !self.pending_card_marks.iter().any(|(_, id, _)| *id == c.id))
            .filter_map(|c| {
                let awaiting = self.prompt.is_some() && Some(c.id) == newest;
                let header = tool_cards::live_header(c, self.tick, awaiting);
                let row = crate::transcript::wrap_lines(&[header], width)
                    .into_iter()
                    .next()?;
                Some((c.start, row))
            })
            .collect()
    }

    /// Toggle the tool card under a screen cell. Returns true when a card was hit, so the caller
    /// treats the click as a card toggle instead of the start of a text selection.
    pub fn toggle_tool_card_at(&mut self, col: u16, row: u16) -> bool {
        let Some((wrapped_row, _)) = self.pointer_to_text(col, row) else {
            return false;
        };
        let width = self.transcript_geom.get().map(|g| g.width).unwrap_or(0);
        self.ensure_wrapped_main(width);
        let source = {
            let cache = self.wrap_cache.borrow();
            match cache.origins.get(wrapped_row) {
                Some(line) => *line,
                None => return false, // the streaming edge, below the committed log
            }
        };
        let Some(id) = self
            .tool_cards
            .iter()
            .find(|c| c.has_detail() && source >= c.start && source < c.start + c.len)
            .map(|c| c.id)
        else {
            return false;
        };
        self.toggle_tool_card(id);
        true
    }

    /// Toggle the most recent expandable card (Ctrl+T) — the keyboard path to the same drawer,
    /// for terminals without mouse reporting and for a quick look at what just ran. A folded
    /// user turn counts: it opens when nothing newer has a drawer.
    pub fn toggle_last_tool_card(&mut self) -> bool {
        let Some(id) = self
            .tool_cards
            .iter()
            .rev()
            .find(|c| c.has_detail())
            .map(|c| c.id)
        else {
            return false;
        };
        self.toggle_tool_card(id);
        true
    }

    fn toggle_tool_card(&mut self, id: u64) {
        if let Some(card) = self.tool_cards.iter_mut().find(|c| c.id == id) {
            card.expanded = !card.expanded;
            self.redraw_tool_card(id);
        }
    }

    /// The rows a user turn is echoed as: the label row, then its text on the user rail. A long
    /// turn folds: a canned command prompt to one labelled row, a long paste to its first rows.
    pub(super) fn echo_user_text(&mut self, text: &str) {
        let label = CANNED
            .iter()
            .find(|(prefix, _)| text.trim_start().starts_with(prefix))
            .map(|(_, label)| label.to_string());
        let cap = (self.last_width.get().max(40) as usize)
            .saturating_sub(6)
            .min(100);
        let rows: Vec<String> = text
            .lines()
            .take(ECHO_LINES)
            .flat_map(|l| {
                let l: String = l.chars().take(ECHO_LINE_CHARS).collect();
                if l.trim().is_empty() {
                    vec![String::new()]
                } else {
                    super::lines::turn::wrap_words(&l, cap)
                }
            })
            .collect();
        let extra = text.lines().count().saturating_sub(ECHO_LINES);
        let long = label.is_some() || rows.len() > USER_FOLD_ROWS;
        if !long {
            let lines: Vec<TextLine<'static>> = rows
                .iter()
                .map(|r| super::lines::turn::user_text(r))
                .collect();
            self.flush.extend(lines);
            return;
        }
        let mut rows = rows;
        if extra > 0 {
            rows.push(format!("… {extra} more lines (sent in full)"));
        }
        if !self.fullscreen {
            let rows = match &label {
                Some(label) => vec![label.clone()],
                None => {
                    let mut kept: Vec<String> = rows.iter().take(USER_FOLD_ROWS).cloned().collect();
                    kept.push(format!(
                        "… {} more lines (sent in full)",
                        rows.len() - USER_FOLD_ROWS
                    ));
                    kept
                }
            };
            self.flush
                .extend(rows.iter().map(|r| super::lines::turn::user_text(r)));
            return;
        }
        let id = self.next_card_id;
        self.next_card_id += 1;
        let card = ToolCard::user_fold(id, UserFold { label, rows });
        let lines = tool_cards::card_lines(&card, self.card_width());
        let len = lines.len();
        self.tool_cards.push(card);
        // The caller tags the whole echo (label, rows, blank) as one user turn.
        self.pending_card_marks.push((self.flush.len(), id, len));
        self.flush.extend(lines);
    }
}

/// A huge paste is sent in full but echoed only in part, so scrollback stays usable.
const ECHO_LINES: usize = 40;
const ECHO_LINE_CHARS: usize = 2000;
