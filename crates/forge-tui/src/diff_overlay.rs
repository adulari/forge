//! The full-screen diff overlay (`/diff`): every changed file of a source, one at a time, with
//! hunk and file navigation, a unified or side-by-side view, and a per-file "reviewed" mark.
//!
//! Two sources: the **working tree** (everything `git diff HEAD` and untracked files show) and
//! **this turn** (the changes the agent made since the last prompt). The host reads git and hands
//! the files in; nothing here touches disk or runs a process.

use std::cell::Cell;
use std::collections::HashSet;

use crate::KeyKind;

mod model;
pub(crate) mod render;

pub use model::{
    from_snapshot, parse_unified, untracked_file, DiffLine, DiffSource, FileStatus, LineKind,
    OverlayFile, OverlayHunk,
};
pub use render::render_diff_overlay;

/// What the host does after a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffAction {
    None,
    /// The working-tree source was selected: read git again.
    Refresh,
    /// The person confirmed reverting one hunk of a working-tree file. The host applies it with
    /// `git apply -R` and refreshes the tree; `header` is the hunk's `@@` line.
    RevertHunk {
        path: String,
        header: String,
        /// The hunk's lines as the overlay shows them (`' '`/`+`/`-` prefixed), so the host can
        /// refuse when the file's hunk under the same header has different content now.
        body: Vec<String>,
    },
}

/// Index of the working-tree source in [`DiffOverlay::sources`].
pub const TREE: usize = 0;
/// Index of the current-turn source.
pub const TURN: usize = 1;

#[derive(Debug, Clone, Default)]
pub struct DiffOverlay {
    pub open: bool,
    sources: Vec<DiffSource>,
    source: usize,
    file: usize,
    scroll: usize,
    pub split: bool,
    reviewed: HashSet<(String, u64)>,
    /// Waiting for the host's git read.
    pub loading: bool,
    /// Why the working tree could not be read (not a repository, git missing).
    pub error: Option<String>,
    view_h: Cell<usize>,
    /// A hunk revert waiting for `y`: `(path, hunk header, hunk body)`.
    confirm_revert: Option<(String, String, Vec<String>)>,
    /// The outcome of the last revert, shown in the hint row until the next key.
    pub notice: Option<String>,
    /// Set by the renderer: the diff column is too narrow for split rows, so unified is drawn.
    narrow: Cell<bool>,
}

/// One drawn row of the current file's body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Row {
    Hunk(usize),
    Line(usize, usize),
    /// A split row: the left (old) and right (new) line of a hunk, either may be empty.
    Pair(usize, Option<usize>, Option<usize>),
}

impl DiffOverlay {
    /// Open on the working tree (loading) with the turn's files already known.
    pub fn open_with(&mut self, turn: Vec<OverlayFile>) {
        self.open = true;
        self.loading = true;
        self.error = None;
        self.sources = vec![
            DiffSource {
                label: "working tree vs HEAD".to_string(),
                files: Vec::new(),
            },
            DiffSource {
                label: "this turn".to_string(),
                files: turn,
            },
        ];
        // A turn that changed files is the answer to "what did it just do"; otherwise the tree is.
        self.source = if self.sources[TURN].files.is_empty() {
            TREE
        } else {
            TURN
        };
        self.file = 0;
        self.scroll = 0;
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    /// The host's git read finished.
    pub fn set_tree(&mut self, result: Result<Vec<OverlayFile>, String>) {
        self.loading = false;
        match result {
            Ok(files) => {
                self.error = None;
                self.sources[TREE].files = files;
            }
            Err(message) => {
                self.error = Some(message);
                self.sources[TREE].files.clear();
            }
        }
        let max = self.files().len().saturating_sub(1);
        self.file = self.file.min(max);
        self.scroll = 0;
    }

    /// The pending revert confirmation, as the hint row shows it.
    pub fn confirm_prompt(&self) -> Option<String> {
        self.confirm_revert
            .as_ref()
            .map(|(path, header, _)| format!("revert hunk {header} in {path}? y/n"))
    }

    /// The hunk at the top of the view, when the working tree is shown and it can be reverted.
    fn revertable_hunk(&self) -> Option<(String, String, Vec<String>)> {
        if self.source != TREE {
            return None;
        }
        let file = self.current()?;
        if file.binary || matches!(file.status, FileStatus::Untracked | FileStatus::Renamed) {
            return None;
        }
        let (_, starts) = self.rows();
        let index = starts.iter().rposition(|&start| start <= self.scroll)?;
        let hunk = file.hunks.get(index)?;
        let body = hunk
            .lines
            .iter()
            .map(|l| {
                let sign = match l.kind {
                    LineKind::Context => ' ',
                    LineKind::Add => '+',
                    LineKind::Del => '-',
                };
                format!("{sign}{}", l.text)
            })
            .collect();
        Some((file.path.clone(), hunk.header.clone(), body))
    }

    /// Read the working tree again (after the host changed it).
    pub fn request_refresh(&mut self) {
        if self.source == TREE {
            self.loading = true;
        }
    }

    pub fn sources(&self) -> &[DiffSource] {
        &self.sources
    }

    pub fn source_index(&self) -> usize {
        self.source
    }

    pub fn file_index(&self) -> usize {
        self.file
    }

    pub fn scroll(&self) -> usize {
        self.scroll
    }

    pub fn files(&self) -> &[OverlayFile] {
        self.sources.get(self.source).map_or(&[], |s| &s.files)
    }

    pub fn current(&self) -> Option<&OverlayFile> {
        self.files().get(self.file)
    }

    pub fn is_reviewed(&self, file: &OverlayFile) -> bool {
        self.reviewed
            .contains(&(file.path.clone(), file.fingerprint()))
    }

    pub fn reviewed_count(&self) -> usize {
        self.files().iter().filter(|f| self.is_reviewed(f)).count()
    }

    pub(crate) fn set_view_height(&self, rows: usize) {
        self.view_h.set(rows);
    }

    pub(crate) fn set_diff_area(&self, rows: usize, narrow: bool) {
        self.view_h.set(rows);
        self.narrow.set(narrow);
    }

    /// The body rows of the current file for the current view mode.
    pub(crate) fn rows(&self) -> (Vec<Row>, Vec<usize>) {
        let Some(file) = self.current() else {
            return (Vec::new(), Vec::new());
        };
        layout(file, self.split && !self.narrow.get())
    }

    fn page(&self) -> usize {
        self.view_h.get().saturating_sub(1).max(1)
    }

    fn clamp_scroll(&mut self) {
        let (rows, _) = self.rows();
        let max = rows.len().saturating_sub(self.view_h.get().max(1));
        self.scroll = self.scroll.min(max);
    }

    fn select_file(&mut self, index: usize) {
        self.file = index;
        self.scroll = 0;
    }

    fn jump_hunk(&mut self, forward: bool) {
        let (_, starts) = self.rows();
        let target = if forward {
            starts.iter().copied().find(|&s| s > self.scroll)
        } else {
            starts.iter().rev().copied().find(|&s| s < self.scroll)
        };
        if let Some(row) = target {
            self.scroll = row;
            self.clamp_scroll();
        }
    }

    /// Mark or unmark the current file, then move to the next file still to review.
    fn toggle_reviewed(&mut self) {
        let Some(file) = self.current() else {
            return;
        };
        let key = (file.path.clone(), file.fingerprint());
        if !self.reviewed.remove(&key) {
            self.reviewed.insert(key);
            let next = (1..self.files().len())
                .map(|step| (self.file + step) % self.files().len())
                .find(|&i| !self.is_reviewed(&self.files()[i]));
            if let Some(i) = next {
                self.select_file(i);
            }
        }
    }

    fn cycle_source(&mut self) -> DiffAction {
        self.source = (self.source + 1) % self.sources.len().max(1);
        self.file = 0;
        self.scroll = 0;
        if self.source == TREE {
            self.loading = true;
            DiffAction::Refresh
        } else {
            DiffAction::None
        }
    }

    /// Handle one key while the overlay is open; it owns every key.
    pub fn key(&mut self, key: KeyKind) -> DiffAction {
        self.notice = None;
        if let Some((path, header, body)) = self.confirm_revert.take() {
            // Any key other than `y` cancels, so a stray press never reverts.
            if key == KeyKind::Char('y') {
                return DiffAction::RevertHunk { path, header, body };
            }
            self.notice = Some("revert cancelled".to_string());
            return DiffAction::None;
        }
        let n = self.files().len();
        match key {
            KeyKind::Esc | KeyKind::Char('q') => self.close(),
            KeyKind::Down | KeyKind::Char('j') => {
                self.scroll += 1;
                self.clamp_scroll();
            }
            KeyKind::Up | KeyKind::Char('k') => self.scroll = self.scroll.saturating_sub(1),
            KeyKind::PageDown | KeyKind::Char(' ') => {
                self.scroll += self.page();
                self.clamp_scroll();
            }
            KeyKind::PageUp => self.scroll = self.scroll.saturating_sub(self.page()),
            KeyKind::Home | KeyKind::Char('g') => self.scroll = 0,
            KeyKind::End | KeyKind::Char('G') => {
                self.scroll = usize::MAX / 2;
                self.clamp_scroll();
            }
            KeyKind::Char(']') => self.jump_hunk(true),
            KeyKind::Char('[') => self.jump_hunk(false),
            KeyKind::Char('n') | KeyKind::Right if n > 0 => self.select_file((self.file + 1) % n),
            KeyKind::Char('p') | KeyKind::Left if n > 0 => {
                self.select_file((self.file + n - 1) % n)
            }
            KeyKind::Char('v') => {
                self.split = !self.split;
                self.scroll = 0;
            }
            KeyKind::Char('d') | KeyKind::Tab => return self.cycle_source(),
            KeyKind::Char('r') if self.source == TREE => {
                self.loading = true;
                return DiffAction::Refresh;
            }
            KeyKind::Char('m') => self.toggle_reviewed(),
            KeyKind::Char('x') => match self.revertable_hunk() {
                Some(target) => self.confirm_revert = Some(target),
                None => {
                    self.notice =
                        Some("hunks can be reverted in the working-tree source".to_string())
                }
            },
            _ => {}
        }
        DiffAction::None
    }
}

/// Lay a file out as rows. `starts` are the rows where each hunk begins.
pub(crate) fn layout(file: &OverlayFile, split: bool) -> (Vec<Row>, Vec<usize>) {
    let mut rows = Vec::new();
    let mut starts = Vec::new();
    for (h, hunk) in file.hunks.iter().enumerate() {
        starts.push(rows.len());
        rows.push(Row::Hunk(h));
        if !split {
            rows.extend((0..hunk.lines.len()).map(|l| Row::Line(h, l)));
            continue;
        }
        let mut i = 0;
        while i < hunk.lines.len() {
            if hunk.lines[i].kind == LineKind::Context {
                rows.push(Row::Pair(h, Some(i), Some(i)));
                i += 1;
                continue;
            }
            let dels: Vec<usize> = (i..hunk.lines.len())
                .take_while(|&j| hunk.lines[j].kind == LineKind::Del)
                .collect();
            let adds_from = i + dels.len();
            let adds: Vec<usize> = (adds_from..hunk.lines.len())
                .take_while(|&j| hunk.lines[j].kind == LineKind::Add)
                .collect();
            for k in 0..dels.len().max(adds.len()) {
                rows.push(Row::Pair(h, dels.get(k).copied(), adds.get(k).copied()));
            }
            i = adds_from + adds.len();
        }
    }
    (rows, starts)
}

impl crate::App {
    /// Open the diff overlay, seeded with this turn's changed files. The host reads the working
    /// tree and hands it in with [`DiffOverlay::set_tree`].
    pub fn open_diff_overlay(&mut self) {
        let turn = self.turn_diffs.iter().map(from_snapshot).collect();
        self.diff_overlay.open_with(turn);
    }
}

#[cfg(test)]
mod tests;
