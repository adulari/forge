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
    pub(super) fn card_is_last(&self, card: &ToolCard) -> bool {
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
    fn foldable_read(&self, parent: Option<u64>) -> Option<u64> {
        let card = self.tool_cards.last()?;
        (card.fold.is_none()
            && card.parent == parent
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
        let parent = self.running_script();
        if self.fullscreen && name == "read_file" {
            if let Some(id) = self.foldable_read(parent) {
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
        card.batch = Some(self.group_for_new_card(id));
        card.parent = parent;
        let lines = tool_cards::card_lines(&card, self.card_width());
        let len = lines.len();
        self.tool_cards.push(card);
        self.seal_flush_origins(&LineOrigin::default());
        self.pending_card_marks.push((self.flush.len(), id, len));
        self.tag_flush(origin, |s| s.flush.extend(lines));
    }

    /// The `run_code` call still running, which a call starting now was made by (a script's
    /// calls run one at a time, inside it). Full-screen only.
    fn running_script(&self) -> Option<u64> {
        if !self.fullscreen {
            return None;
        }
        self.tool_cards
            .iter()
            .rev()
            .find(|c| c.fold.is_none() && c.name == "run_code" && c.status == CardStatus::Running)
            .map(|c| c.id)
    }

    /// Put a shell diagnosis under the failed call it explains, instead of a notice that repeats
    /// the command. False when no failed card ran that command (inline mode prints its cards
    /// once, so it never finds one), and the caller prints the notice.
    pub(super) fn attach_diagnosis(
        &mut self,
        command: &str,
        diagnosis: &str,
        fix: Option<&str>,
    ) -> bool {
        if !self.fullscreen {
            return false;
        }
        let command = command.trim();
        let ran = |args: &str| {
            let v: serde_json::Value = serde_json::from_str(args).unwrap_or_default();
            v.get("command")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                == Some(command)
                || v.get("commands")
                    .and_then(serde_json::Value::as_array)
                    .is_some_and(|l| l.iter().any(|c| c.as_str().map(str::trim) == Some(command)))
        };
        let Some(card) = self.tool_cards.iter_mut().rev().find(|c| {
            c.fold.is_none()
                && c.interactive
                && c.status == CardStatus::Failed
                && c.note.is_none()
                && ran(&c.args)
        }) else {
            return false;
        };
        card.note = Some(tool_cards::CardNote {
            diagnosis: sanitize_terminal_text(diagnosis),
            fix: fix.map(sanitize_terminal_text),
        });
        let id = card.id;
        self.redraw_tool_card(id);
        true
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
        card.set_diff(diff);
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
        // A folded group, or the header of an open one, toggles the group.
        if self.toggle_group_at(col, row) {
            return true;
        }
        let Some((wrapped_row, _)) = self.pointer_to_text(col, row) else {
            return false;
        };
        let width = self.transcript_geom.get().map(|g| g.width).unwrap_or(0);
        self.ensure_wrapped_main(width);
        let source = {
            let cache = self.wrap_cache.borrow();
            let spacer = App::fold_row_at(&cache, wrapped_row)
                .is_some_and(|f| f.kind == super::group_flow::FoldRowKind::Spacer);
            if spacer {
                return false;
            }
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
        // With groups folding, the newest group is the target even when its newest call has no
        // drawer of its own: opening the group is what shows it.
        let grouped = self.fold_enabled();
        let Some(id) = self
            .tool_cards
            .iter()
            .rev()
            .find(|c| c.has_detail() || (grouped && c.batch.is_some() && c.fold.is_none()))
            .map(|c| c.id)
        else {
            return false;
        };
        if !self.cycle_group_of(id) {
            self.toggle_tool_card(id);
        }
        true
    }

    fn toggle_tool_card(&mut self, id: u64) {
        if let Some(card) = self.tool_cards.iter_mut().find(|c| c.id == id) {
            card.expanded = !card.expanded;
            self.redraw_tool_card(id);
        }
    }

    /// The rows a user turn is echoed as: its text on the user rail. A canned command prompt
    /// folds to one labelled row; in full-screen a long paste folds to its first rows (`Ctrl+T`
    /// opens it). Outside full-screen (inline, or the daemon driver a phone reads) a generic
    /// message is never folded: the scrollback scrolls natively and a client must see all of it.
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
        let fold = label.is_some() || (self.fullscreen && rows.len() > USER_FOLD_ROWS);
        if !fold {
            // One row per line the user typed; the transcript wraps them at draw time.
            let mut lines: Vec<TextLine<'static>> = text
                .lines()
                .take(ECHO_LINES)
                .map(|l| {
                    let l: String = l.chars().take(ECHO_LINE_CHARS).collect();
                    super::lines::turn::user_text(&l)
                })
                .collect();
            if extra > 0 {
                lines.push(super::lines::turn::user_text(&format!(
                    "… {extra} more lines (sent in full)"
                )));
            }
            self.flush.extend(lines);
            return;
        }
        let mut rows = rows;
        if extra > 0 {
            rows.push(format!("… {extra} more lines (sent in full)"));
        }
        if !self.fullscreen {
            // Only a canned prompt gets here: one labelled row, nothing to open.
            if let Some(label) = label {
                self.flush.push(super::lines::turn::user_text(&label));
            }
            return;
        }
        let id = self.next_card_id;
        self.next_card_id += 1;
        let hidden: Vec<String> = match label {
            Some(_) => Vec::new(),
            None => rows.iter().skip(USER_FOLD_ROWS).cloned().collect(),
        };
        let card = ToolCard::user_fold(id, UserFold { label, rows });
        let mut lines = tool_cards::card_lines(&card, self.card_width());
        let len = lines.len();
        self.tool_cards.push(card);
        // The caller tags the whole echo (label, rows, blank) as one user turn.
        self.pending_card_marks.push((self.flush.len(), id, len));
        // The fold's hint is for the terminal. A phone reads the whole message: the rows the
        // terminal hides ride along for it.
        let hint = (!hidden.is_empty()).then(|| lines.pop()).flatten();
        self.flush.extend(lines);
        if let Some(hint) = hint {
            self.seal_flush_origins(&LineOrigin::user());
            self.flush.push(hint);
            self.seal_flush_origins(&LineOrigin::local());
        }
        for row in hidden {
            self.remote_only
                .push((super::lines::turn::user_text(&row), LineOrigin::user()));
        }
    }
}

/// A huge paste is sent in full but echoed only in part, so scrollback stays usable.
const ECHO_LINES: usize = 40;
const ECHO_LINE_CHARS: usize = 2000;
