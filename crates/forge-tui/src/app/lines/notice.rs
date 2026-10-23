//! One notice grammar for every one-line message that is not part of the conversation: a glyph
//! at column 1 (`✓` done, `·` information, `▲` warning, `✗` error), the text from column 3, and
//! wrapped lines hanging under the text. Inside an open assistant turn the same notice sits on
//! the rail, two cells in.
//!
//! Notices from forge-cli still arrive as strings that start with a legacy glyph (`⚠ ✖ ⏹ ⚙ ◈ ※
//! →`); [`classify`] reads that glyph into one of the four kinds, so no call site has to change.

use crate::copy_mark;
use crate::theme::styles::{self, NoticeKind};
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use super::rail;

/// Leading glyphs a message may carry, and the kind each one means.
const LEGACY: &[(char, NoticeKind)] = &[
    ('✓', NoticeKind::Ok),
    ('✔', NoticeKind::Ok),
    ('⚠', NoticeKind::Warn),
    ('▲', NoticeKind::Warn),
    ('✖', NoticeKind::Err),
    ('✕', NoticeKind::Err),
    ('✗', NoticeKind::Err),
    ('⏹', NoticeKind::Warn),
    ('⚙', NoticeKind::Info),
    ('◈', NoticeKind::Info),
    ('※', NoticeKind::Info),
    ('→', NoticeKind::Info),
    ('⚒', NoticeKind::Info),
    ('·', NoticeKind::Info),
    ('⋯', NoticeKind::Info),
];

/// Status text that arrives as a warning but is not a problem.
const INFORMATIONAL: &[&str] = &[
    "nothing to compact",
    "completeness check",
    "cli-bridge runs",
    "cli bridge:",
];

/// Split a message into its kind and the text without the leading glyph. `default` is the kind
/// for text with no glyph (a `Warning` event is a warning even without one).
pub(crate) fn classify(text: &str, default: NoticeKind) -> (NoticeKind, String) {
    let trimmed = text.trim_start();
    let mut kind = default;
    let mut body = trimmed;
    if let Some(first) = trimmed.chars().next() {
        if let Some((_, k)) = LEGACY.iter().find(|(g, _)| *g == first) {
            kind = *k;
            body = trimmed[first.len_utf8()..].trim_start();
        }
    }
    let lower = body.to_lowercase();
    if kind == NoticeKind::Warn && INFORMATIONAL.iter().any(|p| lower.starts_with(p)) {
        kind = NoticeKind::Info;
    }
    (kind, crate::theme::glyphs::normalise(body))
}

/// The leading cells of a notice row and of its continuation rows.
fn lead(kind: NoticeKind, in_turn: bool) -> Vec<Span<'static>> {
    let (glyph, style) = styles::notice(kind);
    let mut spans = if in_turn {
        let mut s = rail::rail_spans(rail::BAKED);
        s.push(Span::styled("  ", copy_mark::decor(Style::default())));
        s
    } else {
        vec![Span::styled(" ", copy_mark::decor(Style::default()))]
    };
    spans.push(Span::styled(glyph, copy_mark::decor(style)));
    spans.push(Span::styled(
        " ",
        copy_mark::hang(copy_mark::decor(Style::default())),
    ));
    spans
}

/// The hanging prefix of a detail row: blank under the glyph and gap.
fn hang_row(in_turn: bool, spans: Vec<Span<'static>>) -> Line<'static> {
    let mut row = if in_turn {
        let mut s = rail::rail_spans(rail::BAKED);
        s.push(Span::styled("  ", copy_mark::decor(Style::default())));
        s
    } else {
        vec![Span::styled(" ", copy_mark::decor(Style::default()))]
    };
    row.push(Span::styled(" ", copy_mark::decor(Style::default())));
    row.push(Span::styled(
        " ",
        copy_mark::hang(copy_mark::decor(Style::default())),
    ));
    row.extend(spans);
    Line::from(row)
}

/// A notice: `head` after the glyph, then one muted row per `details` line.
pub(crate) fn notice_with(
    kind: NoticeKind,
    in_turn: bool,
    head: Vec<Span<'static>>,
    details: &[String],
) -> Vec<Line<'static>> {
    // Text that arrives pre-glyphed is folded onto the ASCII set where that is the tier.
    let ascii = |text: String| crate::theme::glyphs::normalise(&text);
    let mut first = lead(kind, in_turn);
    first.extend(
        head.into_iter()
            .map(|s| Span::styled(ascii(s.content.into_owned()), s.style)),
    );
    let mut out = vec![Line::from(first)];
    for d in details {
        out.push(hang_row(
            in_turn,
            vec![Span::styled(ascii(d.clone()), styles::muted())],
        ));
    }
    out
}

/// A plain-text notice (its `\n`s become hanging rows).
pub(crate) fn notice_lines(kind: NoticeKind, in_turn: bool, text: &str) -> Vec<Line<'static>> {
    let mut parts = text.split('\n');
    let head = parts.next().unwrap_or("");
    let rest: Vec<String> = parts.map(str::to_string).collect();
    let (_, glyph_style) = styles::notice(kind);
    // Errors read at full weight; everything else is secondary text.
    let text_style = match kind {
        NoticeKind::Err => styles::err(),
        NoticeKind::Warn => styles::fg(),
        _ => styles::muted(),
    };
    let _ = glyph_style;
    notice_with(
        kind,
        in_turn,
        vec![Span::styled(head.to_string(), text_style)],
        &rest,
    )
}

/// The same notice with a `×N` count after it.
pub(crate) fn counted(kind: NoticeKind, in_turn: bool, text: &str, n: usize) -> Vec<Line<'static>> {
    let mut lines = notice_lines(kind, in_turn, text);
    if let Some(first) = lines.first_mut() {
        first
            .spans
            .push(Span::styled(format!("  ×{n}"), styles::faint()));
    }
    lines
}

/// `Next:` line under an error when a fix is known.
pub(crate) fn next_step(text: &str, in_turn: bool) -> Line<'static> {
    hang_row(
        in_turn,
        vec![
            Span::styled("Next: ", styles::muted().bold()),
            Span::styled(text.to_string(), styles::muted()),
        ],
    )
}

/// Whether an error already tells the person to pick another model: its own `Next:` line would
/// only repeat it.
pub(crate) fn says_pick_another_model(error: &str) -> bool {
    error.to_lowercase().contains("run /model")
}

/// What to try next, for the errors that have one.
pub(crate) fn fix_hint(error: &str) -> Option<&'static str> {
    if says_pick_another_model(error) {
        return None;
    }
    let e = error.to_lowercase();
    let any = |needles: &[&str]| needles.iter().any(|n| e.contains(n));
    if any(&[
        "no keys configured",
        "api key",
        "unauthorized",
        "401",
        "not logged in",
    ]) {
        Some("run `forge doctor` to check your keys")
    } else if any(&["rate limit", "rate-limit", "429", "quota"]) {
        Some("wait a moment, or /model to pick another model")
    } else if any(&[
        "context length",
        "context window",
        "too long",
        "token limit",
    ]) {
        Some("/compact frees up context")
    } else if any(&["model"]) && any(&["not found", "unknown", "does not exist", "unavailable"]) {
        Some("/model lists the models you can use")
    } else if any(&["connection", "timed out", "timeout", "dns", "network"]) {
        Some("check your network, then try again")
    } else {
        None
    }
}

/// A notice's key for collapsing repeats: the text with every run of digits folded to `#`.
pub(crate) fn repeat_key(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_digits = false;
    for c in text.chars() {
        if c.is_ascii_digit() {
            if !in_digits {
                out.push('#');
            }
            in_digits = true;
        } else {
            in_digits = false;
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(lines: &[Line<'_>]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    #[test]
    fn legacy_glyphs_are_classified_into_the_four_kinds() {
        use NoticeKind::*;
        let cases = [
            ("✓ saved", Ok, "saved"),
            ("⚠ careful", Warn, "careful"),
            ("✖ broke", Err, "broke"),
            ("✕ denied", Err, "denied"),
            ("⏹ interrupted", Warn, "interrupted"),
            ("⚙ job 3 exited", Info, "job 3 exited"),
            ("◈ no servers", Info, "no servers"),
            ("※ remember", Info, "remember"),
            ("→ switched model", Info, "switched model"),
            ("plain text", Info, "plain text"),
        ];
        for (input, kind, body) in cases {
            let (k, b) = classify(input, Info);
            assert_eq!((k, b.as_str()), (kind, body), "{input}");
        }
    }

    #[test]
    fn a_warning_event_without_a_glyph_stays_a_warning() {
        let (k, b) = classify("disk is nearly full", NoticeKind::Warn);
        assert_eq!(k, NoticeKind::Warn);
        assert_eq!(b, "disk is nearly full");
    }

    #[test]
    fn status_text_that_arrives_as_a_warning_is_information() {
        for msg in [
            "⚠ nothing to compact yet",
            "▲ completeness check — reviewing the change",
            "⚠ CLI-bridge runs your locally-installed claude",
        ] {
            assert_eq!(classify(msg, NoticeKind::Warn).0, NoticeKind::Info, "{msg}");
        }
    }

    #[test]
    fn a_notice_hangs_its_wrapped_lines_under_the_text() {
        let lines = notice_lines(NoticeKind::Info, false, "first line\nsecond line");
        assert_eq!(text(&lines), vec![" · first line", "   second line"]);
        let wrapped = crate::transcript::wrap_lines(
            &notice_lines(
                NoticeKind::Warn,
                false,
                "alpha beta gamma delta epsilon zeta",
            ),
            16,
        );
        let rows = text(&wrapped);
        assert!(rows.len() >= 3, "{rows:?}");
        assert!(
            rows[1].starts_with("   ") && !rows[1].starts_with("    "),
            "{rows:?}"
        );
    }

    #[test]
    fn inside_a_turn_the_notice_sits_on_the_rail() {
        let lines = notice_lines(NoticeKind::Info, true, "completeness check passed");
        assert_eq!(text(&lines), vec![" ▏   · completeness check passed"]);
    }

    #[test]
    fn repeats_that_differ_only_in_digits_share_a_key() {
        assert_eq!(
            repeat_key("nudge 3 of 5 sent at 12:04"),
            repeat_key("nudge 4 of 5 sent at 12:09")
        );
        assert_ne!(repeat_key("disk full"), repeat_key("disk empty"));
    }
}
