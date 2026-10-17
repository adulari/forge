//! The turn rail: a one-cell bar at column 1 that runs down every row of a turn, with the text
//! hanging at column 3. The rail is decoration (never copied) and repeats on every wrapped row.

use crate::copy_mark;
use crate::theme::glyphs::g;
use crate::theme::styles::{self, RailKind};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

/// The glyph a rail kind draws.
pub(crate) fn glyph(kind: RailKind) -> &'static str {
    match kind {
        RailKind::User => g().rail_user,
        RailKind::Live | RailKind::Settled(_) => g().rail_assistant,
        RailKind::Decision | RailKind::Error => g().rail_decision,
    }
}

/// The three cells in front of every turn row: blank, rail, blank.
pub(crate) fn rail_spans(kind: RailKind) -> Vec<Span<'static>> {
    let gap = copy_mark::repeat(Style::default());
    vec![
        Span::styled(" ", gap),
        Span::styled(glyph(kind), copy_mark::repeat(styles::rail(kind))),
        Span::styled(" ", gap),
    ]
}

/// The rail an assistant row is baked with. Baked rows are settled; fullscreen paints the open
/// turn hot at draw time (see `transcript_flow`).
pub(crate) const BAKED: RailKind = RailKind::Settled(2);

/// Put `prefix` in front of `line`. A tinted row must not paint its prefix cells too.
pub(crate) fn with_prefix(mut prefix: Vec<Span<'static>>, line: Line<'static>) -> Line<'static> {
    if line.style.bg.is_some() {
        for s in &mut prefix {
            s.style = s.style.bg(Color::Reset);
        }
    }
    prefix.extend(line.spans);
    Line {
        spans: prefix,
        style: line.style,
        alignment: line.alignment,
    }
}

/// Put the rail in front of `line`. A blank line becomes a bare rail row, so the bar runs
/// unbroken through the paragraph gaps of a turn.
pub(crate) fn with_rail(line: Line<'static>, kind: RailKind) -> Line<'static> {
    with_prefix(rail_spans(kind), line)
}

pub(crate) fn rail_lines(lines: Vec<Line<'static>>, kind: RailKind) -> Vec<Line<'static>> {
    lines.into_iter().map(|l| with_rail(l, kind)).collect()
}

/// A label row: the rail, the label, and an optional right-aligned detail.
pub(crate) fn label_row(
    kind: RailKind,
    label: Vec<Span<'static>>,
    right: Vec<Span<'static>>,
) -> Line<'static> {
    let mut spans = rail_spans(kind);
    spans.extend(label);
    if !right.is_empty() {
        spans.push(copy_mark::right_marker());
        spans.extend(right);
    }
    Line::from(spans)
}

/// A row of prose under a rail: it wraps at the reading measure and hangs under the rail.
pub(crate) fn text_row(kind: RailKind, text: &str, style: Style) -> Line<'static> {
    let mut spans = rail_spans(kind);
    spans.push(Span::styled(
        String::new(),
        copy_mark::prose(Style::default()),
    ));
    spans.push(Span::styled(text.to_string(), style));
    Line::from(spans)
}

/// Whether `line` is a row under a tool card's header (its `┊` gutter follows the rail).
pub(crate) fn is_card_body(line: &Line<'_>) -> bool {
    line.spans
        .get(4)
        .is_some_and(|s| s.content.as_ref() == g().card_gutter)
        && line.spans.first().is_some_and(crate::copy_mark::is_rail)
}

/// Whether `line` is the assistant's label row (`✦ forge` and the routed model).
pub(crate) fn is_assistant_label(line: &Line<'_>) -> bool {
    line.spans.len() >= 5
        && line.spans[1].content.as_ref() == g().rail_assistant
        && line.spans[3].content.as_ref() == g().brand
        && line.spans[4].content.as_ref() == " forge"
}

/// Whether `line` is the `you` label row of a user turn (for jump-to-previous-message).
pub(crate) fn is_user_label(line: &Line<'_>) -> bool {
    let user_rail = g().rail_user;
    line.spans.len() == 4
        && line.spans[1].content.as_ref() == user_rail
        && line.spans[3].content.as_ref() == "you"
}
