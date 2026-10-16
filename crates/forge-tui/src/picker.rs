//! The interactive session/checkpoint/model picker: a filterable, scrollable selection list the
//! render loop populates from the store and drives with the same keys as the command palette.

/// What an open [`Picker`] is selecting, so the render loop knows what `Enter` does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerKind {
    /// Pick a past session to resume (`/sessions`, `/resume`).
    Sessions,
    /// Pick a checkpoint to rewind to (`/checkpoints`).
    Checkpoints,
    /// Pick the operating mode / temper (`/mode`).
    Tempers,
    /// Pick the Assay action: analysis-only vs full cleanup (`/assay`).
    AssayChoice,
    /// Browse available models (`/models`): a provider list that drills into per-provider models.
    Models,
    /// On resuming a previously-compacted session: continue with the compacted context, or reload
    /// the full original history into the model's view.
    ResumeMode,
    /// Pick what part of an assistant response to copy (`/copy` when the response has code blocks):
    /// the full response or an individual fenced block. Enter → clipboard, `w` → write to a file.
    /// Each row's `id` is the index into [`App::copy_candidates`](crate::App::copy_candidates).
    CopyBlocks,
    /// Flat ranked model picker for `/model` (bare): "mesh (auto)" first, then all known models
    /// sorted by tier (subscription → frontier → paid → free). Enter pins the selected model;
    /// selecting "mesh" clears the pin. Esc closes without changing anything.
    ModelPin,
    /// Step two of pinning: the rated effort rungs of ONE model, best value first and marked.
    ///
    /// Only opened for a model whose ladder has more than one rated rung — for anything else the
    /// list would be a single row restating the pin. Each row's `id` is `provider::model@rung`, so
    /// the loop pins the exact pair without carrying extra state between the two steps.
    ModelRung,
    /// Pick the winning candidate from a finished `/duel` run (docs/features/duel.md). Each row's
    /// `id` is the candidate's worktree branch. Enter merges the winner back and discards the
    /// rest; Esc discards every candidate (no merge, no routing-learning record).
    Duel,
}

/// One row in an interactive picker: an opaque `id` the loop acts on, plus two display strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerRow {
    /// What the loop resolves on Enter (a session id, or a checkpoint boundary seq as a string).
    pub id: String,
    pub title: String,
    pub subtitle: String,
}

fn haystack(row: &PickerRow) -> String {
    format!("{}\n{}\n{}", row.id, row.title, row.subtitle).to_lowercase()
}

/// A full-width, animated, filterable selection list (RFC session-management-and-commands).
/// Reused for `/sessions`, `/resume`, and `/checkpoints` — the loop populates `rows` from the
/// store, then drives it with the same keys as the command palette. Filter narrows as you type.
#[derive(Debug, Clone, Default)]
pub struct Picker {
    pub open: bool,
    pub kind: Option<PickerKind>,
    rows: Vec<PickerRow>,
    /// Lowercased `id\ntitle\nsubtitle` per row, built once when the rows are set. Filtering
    /// runs on every keystroke and every frame, and re-lowercasing three strings per row each time
    /// made a thousand-checkpoint list sluggish.
    haystacks: Vec<String>,
    /// How many leading rows came from `open_with`; later ones were added by `extend_rows`.
    base_len: usize,
    pub query: String,
    pub selected: usize,
    pub anim: f32,
    /// A one-line title shown above the list (e.g. "resume a session").
    pub heading: String,
}

impl Picker {
    pub fn open_with(&mut self, kind: PickerKind, heading: &str, rows: Vec<PickerRow>) {
        self.open = true;
        self.kind = Some(kind);
        self.haystacks = rows.iter().map(haystack).collect();
        self.base_len = rows.len();
        self.rows = rows;
        self.heading = heading.to_string();
        self.query.clear();
        self.selected = 0;
        self.anim = 0.0;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.kind = None;
        self.rows.clear();
        self.haystacks.clear();
        self.base_len = 0;
        self.query.clear();
        self.heading.clear();
        self.selected = 0;
        self.anim = 0.0;
    }

    /// Rows matching the current query (case-insensitive substring over id/title/subtitle).
    pub fn matches(&self) -> Vec<&PickerRow> {
        if self.query.is_empty() {
            return self.rows.iter().collect();
        }
        let q = self.query.to_lowercase();
        self.rows
            .iter()
            .zip(&self.haystacks)
            .filter(|(_, hay)| hay.contains(&q))
            .map(|(row, _)| row)
            .collect()
    }

    /// Drop rows added by [`Self::extend_rows`], keeping the ones the picker was opened with.
    pub fn reset_to_base(&mut self) {
        self.rows.truncate(self.base_len);
        self.haystacks.truncate(self.base_len);
    }

    /// Append rows found after opening (a full-text hit the opening list did not carry). They are
    /// still subject to the filter, so a row only shows if its text carries the query.
    pub fn extend_rows(&mut self, rows: Vec<PickerRow>) {
        self.haystacks.extend(rows.iter().map(haystack));
        self.rows.extend(rows);
    }

    /// Rows in the list as opened, before any [`Self::extend_rows`].
    pub fn base_len(&self) -> usize {
        self.base_len
    }

    /// The rows the picker was opened with, whatever the filter currently shows.
    pub fn base_rows(&self) -> &[PickerRow] {
        &self.rows[..self.base_len.min(self.rows.len())]
    }

    /// Jump to the first row.
    pub fn home(&mut self) {
        self.selected = 0;
    }

    /// Jump to the last (filtered) row.
    pub fn end(&mut self) {
        self.selected = self.matches().len().saturating_sub(1);
    }

    /// Move the selection up by a page of `rows`.
    pub fn page_up(&mut self, rows: usize) {
        self.selected = self.selected.saturating_sub(rows.max(1));
    }

    /// Move the selection down by a page of `rows`, stopping on the last (filtered) row.
    pub fn page_down(&mut self, rows: usize) {
        let n = self.matches().len();
        if n > 0 {
            self.selected = (self.selected + rows.max(1)).min(n - 1);
        }
    }

    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn move_down(&mut self) {
        let n = self.matches().len();
        if n > 0 {
            self.selected = (self.selected + 1).min(n - 1);
        }
    }

    /// Re-clamp the selection after the filtered list shrinks.
    pub fn clamp(&mut self) {
        let n = self.matches().len();
        if n == 0 {
            self.selected = 0;
        } else if self.selected >= n {
            self.selected = n - 1;
        }
    }

    /// The selected row, if any (after filtering).
    pub fn selected_row(&self) -> Option<&PickerRow> {
        self.matches().into_iter().nth(self.selected)
    }

    pub fn tick_anim(&mut self) {
        if self.open && self.anim < 1.0 {
            self.anim = (self.anim + 0.34).min(1.0);
        }
    }
}
