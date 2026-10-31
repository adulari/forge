//! The system-prompt addendum for `[harness] edit_mode = "hashline"`.

use crate::Session;

/// The base prompt's Tools section describes exact-text `edit_file`/`multi_edit`; in hashline mode
/// that is wrong, so this is appended to say what replaces it.
pub(crate) const HASHLINE_NOTE: &str = "\n\nFile editing in this session: read_file returns \
`[path#TAG]` then `N:text` rows. Use edit_file with that path and tag and `edits` of line numbers \
(`{start, end?, new}` or `{after, new}`), not exact `old` text; ignore the instruction above about \
`old` and multi_edit. Line numbers are from your last read of the file; after an edit use the tag \
and rows it returns.";

impl Session {
    /// The addendum when this session edits by line anchor.
    pub(crate) fn edit_mode_note(&self) -> &'static str {
        if self.config.harness.edit_mode.is_hashline() {
            HASHLINE_NOTE
        } else {
            ""
        }
    }
}
