//! The key hints in the board's top row: right-aligned between the header chips and the
//! connection indicator. Hints are dropped whole from the end (lowest priority first), so the
//! title and the urgent keys at the front are never clipped.

use ratatui::layout::Rect;
use ratatui::Frame;

use super::widgets::{self as w, Bind};
use crate::theme::styles;

const GAP: u16 = 2;

fn bind_width(b: &Bind) -> u16 {
    (w::cells(b.0) + 1 + w::cells(b.1)) as u16
}

/// The longest prefix of `binds` that fits in `width` cells, joined by ` · `.
pub(super) fn fit_binds(binds: &[Bind], width: u16) -> &[Bind] {
    let mut used = 0u16;
    for (i, b) in binds.iter().enumerate() {
        let need = bind_width(b) + if i > 0 { 3 } else { 0 };
        if used + need > width {
            return &binds[..i];
        }
        used += need;
    }
    binds
}

/// Draw the hints into the free span `[left_end, right_start)` of the header row `area`.
pub(super) fn draw_top_hints(
    frame: &mut Frame,
    area: Rect,
    binds: &[Bind],
    left_end: u16,
    right_start: u16,
) {
    let free = right_start.saturating_sub(left_end).saturating_sub(GAP * 2);
    let shown = fit_binds(binds, free);
    if shown.is_empty() {
        return;
    }
    let total: u16 = shown.iter().map(bind_width).sum::<u16>() + 3 * (shown.len() as u16 - 1);
    let mut row = w::Row::at(right_start - GAP - total, area.y);
    for (i, (key, desc, urgent)) in shown.iter().enumerate() {
        if i > 0 {
            row.sep();
        }
        let key_style = if *urgent {
            styles::err().bold()
        } else {
            styles::user().bold()
        };
        row.add(*key, key_style);
        row.add(" ", ratatui::style::Style::default());
        row.add(*desc, styles::muted());
    }
    row.render(frame, area);
}

#[cfg(test)]
mod tests {
    use super::*;

    const BINDS: [Bind; 3] = [
        ("y", "allow", true),
        ("Enter", "open", false),
        ("q", "quit", false),
    ];

    #[test]
    fn lowest_priority_hints_drop_first() {
        assert_eq!(fit_binds(&BINDS, 100).len(), 3);
        assert_eq!(fit_binds(&BINDS, 20).len(), 2);
        assert_eq!(fit_binds(&BINDS, 7).len(), 1);
        assert_eq!(fit_binds(&BINDS, 3).len(), 0);
    }
}
