//! The queued follow-ups, above the composer.
//!
//! A message typed while a turn runs leaves the composer at once, so without this band it would
//! vanish: the only trace was a `1 queued` chip on the border. The band lists what is waiting,
//! dimmed and one line each, and says which keys act on it. It is a live-region row, never
//! scrollback, so it goes away the moment the message is delivered.

use unicode_width::UnicodeWidthStr;

use super::list_view::fit;
use super::*;
use crate::theme::glyphs::g;
use crate::theme::styles;

/// Queued messages listed at most; the earlier ones are counted in the header.
const MAX_ROWS: usize = 3;

/// The band as lines for `width` columns: a header with the keys that work now, then the newest
/// queued messages in arrival order. Empty when nothing is queued.
pub(crate) fn lines(app: &App, width: u16) -> Vec<TextLine<'static>> {
    if app.queued.is_empty() {
        return Vec::new();
    }
    let glyphs = g();
    let width = width as usize;
    let total = app.queued.len();
    let count = format!(" {total} queued");
    // The keys act on the composer and the turn; while a decision block owns the keyboard they
    // would be a promise the band cannot keep.
    let decision_open = app.prompt.is_some() || app.awaiting_question();
    let mut hints: Vec<Vec<(&str, &str)>> = Vec::new();
    if !decision_open {
        if app.input.is_empty() {
            hints.push(vec![(glyphs.up, "edit")]);
        }
        if app.busy {
            // The longest wording that fits wins: the key stays, its explanation shortens.
            hints.push(vec![("Esc", "stop and send now"), ("Esc", "send now")]);
        }
    }
    let sep = format!(" {} ", glyphs.sep);
    let mut header = vec![Span::styled(count.clone(), styles::warn())];
    let mut used = count.width();
    for options in hints {
        let fitted = options
            .into_iter()
            .find(|(key, label)| used + sep.width() + key.width() + 1 + label.width() < width);
        let Some((key, label)) = fitted else {
            break;
        };
        used += sep.width() + key.width() + 1 + label.width();
        header.push(Span::styled(sep.clone(), styles::faint()));
        header.extend(styles::hint_spans(key, label));
    }
    let mut out = vec![TextLine::from(header)];
    let shown = total.min(MAX_ROWS);
    for text in &app.queued[total - shown..] {
        let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let prefix = format!("   {} ", glyphs.prompt);
        let room = width.saturating_sub(prefix.width() + 1).max(1);
        out.push(TextLine::from(vec![
            Span::styled(prefix, styles::faint()),
            Span::styled(fit(&flat, room), styles::muted()),
        ]));
    }
    out
}

/// Rows the band takes, given the `avail` rows the layout can spare.
pub(crate) fn height(app: &App, width: u16, avail: u16) -> u16 {
    (lines(app, width).len() as u16).min(avail)
}

/// Draw the band into `area` (whose height is what [`height`] returned). A short area keeps the
/// header and the newest messages, which are the ones `↑` pulls back.
pub(crate) fn render(frame: &mut Frame, area: Rect, app: &App) {
    if area.height == 0 {
        return;
    }
    let mut all = lines(app, area.width);
    let room = area.height as usize;
    if all.len() > room {
        let header = all.remove(0);
        let keep = room.saturating_sub(1);
        let tail = all.split_off(all.len() - keep);
        all = std::iter::once(header).chain(tail).collect();
    }
    frame.render_widget(Paragraph::new(all), area);
}
