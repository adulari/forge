//! The empty-transcript welcome state. Drawn by the panels surface; this is its seam.

use super::*;

/// Draw the welcome into `area` when the transcript is empty. Returns whether it drew anything,
/// so the caller falls back to the transcript renderer otherwise. Not drawn yet.
pub(crate) fn render_empty_state(_frame: &mut Frame, _area: Rect, _app: &App) -> bool {
    false
}
