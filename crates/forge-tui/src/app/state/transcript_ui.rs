//! UI state owned by the transcript surface (turn rails, folded rows, notice collapsing).

/// State only the transcript renderer reads or writes. Starts empty; the transcript work package
/// adds fields here and nowhere else.
#[derive(Debug, Clone, Default)]
pub(crate) struct TranscriptUi {}
