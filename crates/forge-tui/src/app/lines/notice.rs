//! Scrollback line builder for plain command-feedback notes.

use crate::theme::styles;
use ratatui::text::{Line as TextLine, Span};

pub(crate) fn note_line(text: &str) -> TextLine<'static> {
    TextLine::from(Span::styled(format!("  {text}"), styles::muted()))
}
