//! Incremental markdown for a reply that is still streaming.
//!
//! Re-parsing, re-highlighting and re-wrapping the whole reply for every refresh made each frame
//! cost O(reply): a 20k-token answer stalled the UI for 30-60 ms every few frames, and typing
//! stalled with it. A finished block never changes, so the reply is split at block boundaries
//! that no later text can reach back across; finished blocks are rendered and wrapped once and
//! only the open block at the end is re-rendered as tokens arrive.
//!
//! The same [`Renderer`] state carries across blocks, so the joined output is line-for-line what
//! [`markdown_to_lines`](super::markdown_to_lines) produces for the whole text (pinned by tests).

use pulldown_cmark::Parser;
use ratatui::text::Line;

use super::{markdown_options, Renderer};
use crate::transcript::wrap_lines;

/// Re-render the open block on every refresh while it is this small; beyond it the refresh rate
/// is bounded by [`REPARSE_INTERVAL`] / [`REPARSE_BYTES`] so a huge open code block cannot make
/// each token cost a full highlight pass.
const SMALL_TAIL_BYTES: usize = 2048;
/// Text committed per refresh. A burst (a proxy flushing a buffered stream, a stalled connection
/// catching up) can deliver most of a reply in one go; rendering it in one frame would stall the
/// UI for the whole parse, so it is taken in slices of about this size, one per frame.
const COMMIT_SLICE_BYTES: usize = 8 * 1024;
/// While finished blocks are still being caught up on, an open tail longer than this is shown as
/// plain text instead of being parsed (and re-parsed every frame) until the slices reach it.
const MAX_TAIL_PARSE_BYTES: usize = 16 * 1024;
const CATCH_UP_GAP: std::time::Duration = std::time::Duration::from_millis(8);
const REPARSE_INTERVAL: std::time::Duration = std::time::Duration::from_millis(80);
const REPARSE_BYTES: usize = 256;

/// Load the bundled syntaxes and theme now (they take tens of milliseconds), so the first code
/// block of a reply does not stall a frame. Meant for a background thread at TUI start.
pub fn prewarm_highlighter() {
    let _ = super::highlighter();
}

#[cfg(test)]
thread_local! {
    static PARSED_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(super) fn note_parsed(bytes: usize) {
    PARSED_BYTES.with(|n| n.set(n.get() + bytes));
}

impl Renderer {
    /// Render `md` as a self-contained run of blocks onto the end of `lines`, leaving only the
    /// output behind (every open construct is closed when the parser reaches the end).
    fn push_markdown(&mut self, md: &str) {
        #[cfg(test)]
        note_parsed(md.len());
        for ev in Parser::new_ext(md, markdown_options()) {
            self.event(ev);
        }
        self.flush_line();
        let lines = std::mem::take(&mut self.lines);
        *self = Renderer {
            lines,
            ..Renderer::default()
        };
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockKind {
    /// Paragraph, heading, table, rule: ends at the next blank line and nothing later joins it.
    Plain,
    /// List item, block quote, indented code, raw HTML: later lines can continue it across blank
    /// lines, so it only ends once an unindented line of a different kind shows up.
    Continuable,
}

fn leading_spaces(line: &str) -> usize {
    line.bytes().take_while(|b| *b == b' ').count()
}

fn is_list_marker(trimmed: &str) -> bool {
    let bytes = trimmed.as_bytes();
    match bytes.first() {
        Some(b'-' | b'*' | b'+') => matches!(bytes.get(1), Some(b' ' | b'\t') | None),
        Some(b'0'..=b'9') => {
            let digits = bytes.iter().take_while(|b| b.is_ascii_digit()).count();
            digits <= 9
                && matches!(bytes.get(digits), Some(b'.' | b')'))
                && matches!(bytes.get(digits + 1), Some(b' ' | b'\t') | None)
        }
        _ => false,
    }
}

fn classify(line: &str) -> BlockKind {
    if line.starts_with('\t') || leading_spaces(line) >= 4 {
        return BlockKind::Continuable;
    }
    let trimmed = line.trim_start();
    if trimmed.starts_with('>') || trimmed.starts_with('<') || is_list_marker(trimmed) {
        BlockKind::Continuable
    } else {
        BlockKind::Plain
    }
}

/// An opening code fence: its character and run length.
fn fence_open(line: &str) -> Option<(u8, usize)> {
    if leading_spaces(line) > 3 {
        return None;
    }
    let trimmed = line.trim_start_matches(' ');
    let ch = *trimmed.as_bytes().first()?;
    if ch != b'`' && ch != b'~' {
        return None;
    }
    let run = trimmed.bytes().take_while(|b| *b == ch).count();
    if run < 3 {
        return None;
    }
    // A backtick fence's info string may not contain a backtick.
    if ch == b'`' && trimmed[run..].contains('`') {
        return None;
    }
    Some((ch, run))
}

fn fence_closes(line: &str, ch: u8, run: usize) -> bool {
    if leading_spaces(line) > 3 {
        return false;
    }
    let trimmed = line.trim();
    trimmed.len() >= run && trimmed.bytes().all(|b| b == ch)
}

/// Byte offset in `text` of the last point after `from` at which everything before is a run of
/// complete blocks and nothing after can change how they render, or `from` if there is none.
/// Only complete (newline-terminated) lines are considered.
#[cfg(test)]
pub(super) fn safe_boundary(text: &str, from: usize) -> usize {
    safe_boundary_within(text, from, usize::MAX).0
}

/// [`safe_boundary`], stopping once a boundary at least `budget` bytes past `from` is found. The
/// flag says the scan stopped early, so more boundaries may follow.
pub(super) fn safe_boundary_within(text: &str, from: usize, budget: usize) -> (usize, bool) {
    let mut boundary = from;
    let mut fence: Option<(u8, usize)> = None;
    // The block a fence was opened inside: a fence nested in (or directly after) a list item
    // leaves the list open, anything else is a block of its own.
    let mut fence_outer: Option<BlockKind> = None;
    // The block currently being read, and a blank line seen after a continuable one (committed
    // only once the next unindented, non-continuing line proves the block is over).
    let mut kind: Option<BlockKind> = None;
    let mut pending: Option<usize> = None;
    let mut offset = from;
    for raw in text[from..].split_inclusive('\n') {
        if !raw.ends_with('\n') {
            break;
        }
        if boundary - from >= budget {
            return (boundary, true);
        }
        let end = offset + raw.len();
        let line = raw.trim_end_matches(['\n', '\r']);
        offset = end;
        if let Some((ch, run)) = fence {
            if fence_closes(line, ch, run) {
                fence = None;
                if fence_outer != Some(BlockKind::Continuable) {
                    boundary = end;
                    kind = None;
                }
            }
            continue;
        }
        if line.trim().is_empty() {
            match kind {
                Some(BlockKind::Plain) => {
                    boundary = end;
                    kind = None;
                }
                Some(BlockKind::Continuable) => pending = Some(end),
                None => {}
            }
            continue;
        }
        if let Some(at) = pending.take() {
            let continues = leading_spaces(line) > 0
                || line.starts_with('\t')
                || line.trim_start().starts_with('>')
                || is_list_marker(line.trim_start());
            if !continues {
                boundary = at;
                kind = None;
            }
        }
        if let Some(open) = fence_open(line) {
            fence = Some(open);
            fence_outer = kind;
            kind.get_or_insert(BlockKind::Plain);
            continue;
        }
        kind.get_or_insert_with(|| classify(line));
    }
    (boundary, false)
}

/// A streaming reply rendered as finished blocks plus a re-rendered open tail.
#[derive(Default)]
pub(crate) struct StreamMarkdown {
    renderer: Renderer,
    committed_bytes: usize,
}

impl std::fmt::Debug for StreamMarkdown {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamMarkdown")
            .field("committed_bytes", &self.committed_bytes)
            .field("committed_lines", &self.renderer.lines.len())
            .finish()
    }
}

impl StreamMarkdown {
    pub(crate) fn committed_bytes(&self) -> usize {
        self.committed_bytes
    }

    pub(crate) fn committed_lines(&self) -> &[Line<'static>] {
        &self.renderer.lines
    }

    /// Render every newly finished block of `text` (which only ever grows) and keep it.
    pub(crate) fn advance(&mut self, text: &str) {
        self.advance_within(text, usize::MAX);
    }

    /// [`advance`](Self::advance), but taking roughly `budget` bytes at most (whole blocks only).
    /// Returns whether finished blocks remain to be taken.
    pub(crate) fn advance_within(&mut self, text: &str, budget: usize) -> bool {
        let (boundary, more) = safe_boundary_within(text, self.committed_bytes, budget);
        if boundary > self.committed_bytes {
            self.renderer
                .push_markdown(&text[self.committed_bytes..boundary]);
            self.committed_bytes = boundary;
        }
        more
    }

    /// The open block as lines, without keeping them. Trailing blank lines are dropped, as the
    /// whole-document renderer does.
    pub(crate) fn tail_lines(&mut self, text: &str) -> Vec<Line<'static>> {
        let rest = &text[self.committed_bytes.min(text.len())..];
        if rest.is_empty() {
            return Vec::new();
        }
        let base = self.renderer.lines.len();
        self.renderer.push_markdown(rest);
        let mut tail = self.renderer.lines.split_off(base);
        while matches!(tail.last(), Some(l) if l.spans.is_empty()) {
            tail.pop();
        }
        tail
    }

    /// Everything, exactly as [`markdown_to_lines`](super::markdown_to_lines) would render `text`
    /// — but only the open block is parsed, the finished ones were rendered as they arrived.
    pub(crate) fn finish(mut self, text: &str) -> Vec<Line<'static>> {
        self.advance(text);
        let rest = &text[self.committed_bytes.min(text.len())..];
        if !rest.is_empty() {
            self.renderer.push_markdown(rest);
        }
        let mut lines = self.renderer.lines;
        while matches!(lines.last(), Some(l) if l.spans.is_empty()) {
            lines.pop();
        }
        lines
    }
}

/// The wrapped rows the transcript shows for the in-flight reply. Derived data: a clone starts
/// empty and rebuilds from the reply text on its first refresh.
#[derive(Debug, Default)]
pub(crate) struct StreamCache {
    rev: u64,
    /// Bytes of the reply the rendered rows cover; text past it is shown as plain rows.
    len: usize,
    width: u16,
    rendered_at: Option<std::time::Instant>,
    md: StreamMarkdown,
    committed_rows: Vec<Line<'static>>,
    wrapped_committed: usize,
    /// Finished blocks were left untaken by the last refresh (a burst is being caught up on).
    catching_up: bool,
    tail_rows: Vec<Line<'static>>,
}

impl Clone for StreamCache {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl StreamCache {
    pub(crate) fn len(&self) -> usize {
        self.len
    }

    /// Rows to show: finished rows, plus the open block. A trailing blank row is hidden while
    /// there is no open block so the cursor sits on the last line of text.
    fn finished_rows(&self) -> usize {
        let mut n = self.committed_rows.len();
        if self.tail_rows.is_empty() {
            while n > 0
                && self.committed_rows[n - 1]
                    .spans
                    .iter()
                    .all(|s| s.content.is_empty())
            {
                n -= 1;
            }
        }
        n
    }

    pub(crate) fn row_count(&self) -> usize {
        self.finished_rows() + self.tail_rows.len()
    }

    pub(crate) fn row(&self, i: usize) -> Option<&Line<'static>> {
        let finished = self.finished_rows();
        if i < finished {
            self.committed_rows.get(i)
        } else {
            self.tail_rows.get(i - finished)
        }
    }

    /// Rows `start..end` (clamped), cloned.
    pub(crate) fn window(&self, start: usize, end: usize) -> Vec<Line<'static>> {
        let end = end.min(self.row_count());
        (start.min(end)..end)
            .filter_map(|i| self.row(i).cloned())
            .collect()
    }

    /// Bring the rows up to date with `text`. Cheap when nothing changed or when only a few
    /// bytes arrived for a large open block; a large backlog is taken in slices over several
    /// calls.
    pub(crate) fn refresh(&mut self, text: &str, rev: u64, width: u16) {
        if self.md.committed_bytes() > text.len() || (text.is_empty() && self.len > 0) {
            *self = Self::default();
        }
        let width_changed = self.width != width;
        let changed = self.rev != rev;
        let open_bytes = text.len().saturating_sub(self.md.committed_bytes());
        let since = self.rendered_at.map(|at| at.elapsed());
        let due = since.is_none_or(|d| d >= REPARSE_INTERVAL)
            || text.len().saturating_sub(self.len) >= REPARSE_BYTES
            || open_bytes <= SMALL_TAIL_BYTES;
        let catching_up = self.catching_up && since.is_none_or(|d| d >= CATCH_UP_GAP);
        if !width_changed && !(changed && due) && !catching_up {
            return;
        }
        let wrap_width = width.saturating_sub(1) as usize;
        if width_changed {
            self.committed_rows.clear();
            self.wrapped_committed = 0;
        }
        self.catching_up = self.md.advance_within(text, COMMIT_SLICE_BYTES);
        let committed = self.md.committed_lines();
        if self.wrapped_committed < committed.len() {
            self.committed_rows
                .extend(wrap_lines(&committed[self.wrapped_committed..], wrap_width));
            self.wrapped_committed = committed.len();
        }
        let rest = &text[self.md.committed_bytes().min(text.len())..];
        let tail = if self.catching_up && rest.len() > MAX_TAIL_PARSE_BYTES {
            rest.split('\n').map(|l| Line::raw(l.to_string())).collect()
        } else {
            self.md.tail_lines(text)
        };
        self.tail_rows = wrap_lines(&tail, wrap_width);
        self.rev = rev;
        self.len = text.len();
        self.width = width;
        self.rendered_at = Some(std::time::Instant::now());
    }

    /// The finished reply's lines, reusing every block already rendered. Consumes the cache.
    pub(crate) fn finish(self, text: &str) -> Vec<Line<'static>> {
        self.md.finish(text)
    }
}

#[cfg(test)]
mod tests;
