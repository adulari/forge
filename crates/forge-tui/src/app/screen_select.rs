//! Screen-cell text selection for everything that is not the plain transcript: popups, pickers,
//! the activity and full-output viewers. Mouse capture hides the terminal's own selection, and the
//! transcript selection only knows wrapped-log coordinates, so a drag over an overlay is tracked in
//! screen cells and read back from a snapshot of the frame that was actually drawn. Nothing is
//! overlay-specific: whatever is on screen is what gets copied.

use super::*;
use ratatui::buffer::Buffer;
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ScreenSel {
    anchor: (u16, u16),
    cursor: (u16, u16),
    /// The framed surface the drag started in (interior only), so a row never picks up the chat
    /// visible beside a centered popup. `None` for full-frame surfaces.
    clip: Option<Rect>,
    /// Started over an overlay, so closing it drops the highlight.
    overlay: bool,
}

/// One copyable cell: its glyph, and whether it is decoration (see [`crate::copy_mark`]).
#[derive(Debug, Clone, Default)]
pub(crate) struct ScreenSnapshot {
    rows: Vec<Vec<(String, bool)>>,
}

const BORDER_EDGE: &[char] = &['│', '┃', '║', '┆', '┇', '╎', '╏'];
const BORDER_ANY: &[char] = &[
    '│', '┃', '║', '┆', '┇', '╎', '╏', '─', '━', '═', '╭', '╮', '╰', '╯', '┌', '┐', '└', '┘', '├',
    '┤', '┬', '┴', '┼',
];

impl App {
    pub(super) fn overlay_open(&self) -> bool {
        self.modal_surface_open()
            || self.viewer.is_some()
            || self.workflow.open
            || self.form.is_some()
            || self.effort_slider
    }

    /// Whether a left-press at this cell should start a screen selection instead of a transcript
    /// one: an overlay covers the transcript, or the cell is outside the transcript area.
    pub fn screen_select_wanted(&self, col: u16, row: u16) -> bool {
        self.overlay_open() || self.pointer_to_text(col, row).is_none()
    }

    pub fn screen_selection_active(&self) -> bool {
        self.screen_sel.get().is_some()
    }

    pub fn screen_select_begin(&mut self, col: u16, row: u16) {
        let clip = self
            .panel_rects
            .borrow()
            .iter()
            .rev()
            .find(|r| col >= r.x && col < r.x + r.width && row >= r.y && row < r.y + r.height)
            .map(|r| {
                Rect::new(
                    r.x + 1,
                    r.y + 1,
                    r.width.saturating_sub(2),
                    r.height.saturating_sub(2),
                )
            })
            .filter(|r| r.width > 0 && r.height > 0);
        let clip = clip
            .filter(|r| col >= r.x && col < r.x + r.width && row >= r.y && row < r.y + r.height);
        self.screen_sel.set(Some(ScreenSel {
            anchor: (col, row),
            cursor: (col, row),
            clip,
            overlay: self.overlay_open(),
        }));
    }

    pub fn screen_select_extend(&mut self, col: u16, row: u16) {
        let Some(mut sel) = self.screen_sel.get() else {
            return;
        };
        let (col, row) = match sel.clip {
            Some(c) => (
                col.clamp(c.x, c.x + c.width - 1),
                row.clamp(c.y, c.y + c.height - 1),
            ),
            None => (col, row),
        };
        sel.cursor = (col, row);
        self.screen_sel.set(Some(sel));
    }

    /// Finish the drag. Returns the text to copy; a press without movement copies nothing and
    /// leaves no highlight.
    pub fn screen_select_end(&mut self) -> Option<String> {
        let sel = self.screen_sel.get()?;
        if sel.anchor == sel.cursor {
            self.screen_sel.set(None);
            return None;
        }
        self.screen_selection_text()
    }

    pub fn clear_screen_selection(&self) {
        self.screen_sel.set(None);
    }

    /// Ordered `(first, last)` cells of the drag, by row then column.
    fn screen_span(sel: &ScreenSel) -> ((u16, u16), (u16, u16)) {
        let (a, b) = (sel.anchor, sel.cursor);
        if (a.1, a.0) <= (b.1, b.0) {
            (a, b)
        } else {
            (b, a)
        }
    }

    /// The half-open column range selected on `row`, or `None` when the row is outside the drag.
    fn screen_row_cols(sel: &ScreenSel, row: u16, width: u16) -> Option<(u16, u16)> {
        let ((c0, r0), (c1, r1)) = Self::screen_span(sel);
        if row < r0 || row > r1 {
            return None;
        }
        let (lo, hi) = match sel.clip {
            Some(c) => {
                if row < c.y || row >= c.y + c.height {
                    return None;
                }
                (c.x, c.x + c.width)
            }
            None => (0, width),
        };
        let start = if row == r0 { c0.max(lo) } else { lo };
        let end = if row == r1 { (c1 + 1).min(hi) } else { hi };
        (start < end).then_some((start, end))
    }

    /// Called at the end of every draw: forget a stale selection, keep the overlay rectangles for
    /// the next press, snapshot the frame while a drag is live, and paint the highlight.
    pub(crate) fn finish_frame(&self, buf: &mut Buffer) {
        *self.panel_rects.borrow_mut() = surface::take_panels();
        let Some(sel) = self.screen_sel.get() else {
            return;
        };
        if sel.overlay && !self.overlay_open() {
            self.screen_sel.set(None);
            return;
        }
        let area = buf.area;
        *self.screen_snap.borrow_mut() = ScreenSnapshot {
            rows: (area.y..area.y + area.height)
                .map(|y| {
                    (area.x..area.x + area.width)
                        .map(|x| {
                            let cell = &buf[(x, y)];
                            (
                                cell.symbol().to_string(),
                                crate::copy_mark::is_decor(cell.modifier),
                            )
                        })
                        .collect()
                })
                .collect(),
        };
        if sel.anchor == sel.cursor {
            return;
        }
        for y in area.y..area.y + area.height {
            if let Some((start, end)) = Self::screen_row_cols(&sel, y, area.x + area.width) {
                for x in start..end.min(area.x + area.width) {
                    buf[(x, y)].set_style(crate::theme::styles::selection());
                }
            }
        }
    }

    fn screen_selection_text(&self) -> Option<String> {
        let sel = self.screen_sel.get()?;
        let snap = self.screen_snap.borrow();
        let width = snap.rows.first().map_or(0, |r| r.len() as u16);
        let ((_, first), (_, last)) = Self::screen_span(&sel);
        let rows = (first..=last).filter_map(|y| {
            let cells = snap.rows.get(y as usize)?;
            let (start, end) = Self::screen_row_cols(&sel, y, width)?;
            Some(snapshot_row_text(
                cells,
                start as usize,
                end as usize,
                sel.clip.is_none(),
            ))
        });
        let text = join_rows(rows.collect());
        (!text.trim().is_empty()).then_some(text)
    }
}

fn snapshot_row_text(
    cells: &[(String, bool)],
    start: usize,
    end: usize,
    strip_borders: bool,
) -> String {
    let mut out = String::new();
    let mut x = start;
    while x < end.min(cells.len()) {
        let (sym, decor) = &cells[x];
        if !decor {
            out.push_str(sym);
        }
        x += sym.width().max(1);
    }
    let text = out.trim_end();
    if !strip_borders {
        return text.to_string();
    }
    if text.chars().all(|c| c == ' ' || BORDER_ANY.contains(&c)) {
        return String::new();
    }
    text.trim_matches(|c: char| c == ' ' || BORDER_EDGE.contains(&c))
        .to_string()
}

fn join_rows(mut rows: Vec<String>) -> String {
    while rows.last().is_some_and(|r| r.is_empty()) {
        rows.pop();
    }
    let lead = rows.iter().take_while(|r| r.is_empty()).count();
    rows.drain(..lead);
    rows.join("\n")
}

#[cfg(test)]
#[path = "screen_select_tests.rs"]
mod tests;
