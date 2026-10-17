//! Scrollback line builders: plan.

use crate::theme::styles;
use ratatui::style::Style;
use ratatui::text::{Line as TextLine, Span};

use crate::app::lines::turn::wrap_words;

/// Render a proposed plan (the `present_plan` tool) as a styled card flushed to scrollback: an
/// orange frame + "⬡ PLAN" title, cyan step numbers with a green ❯ marker, dim per-step detail,
/// an optional yellow notes line, and a dim footer hinting the interactive approve prompt that
/// follows. The frame is sized to the widest row (clamped) so it stays tidy at any plan length.
pub(crate) fn plan_card_lines(plan: &forge_types::PlanProposal) -> Vec<TextLine<'static>> {
    let frame = crate::copy_mark::decor(styles::brand());
    let title = plan.title.trim().to_string();

    // Each logical row is a fixed prefix (number/marker/indent) plus a wrappable text payload.
    // The payload is word-wrapped to the inner width so long step titles/details stay inside the
    // frame instead of overflowing the right border.
    struct Row {
        prefix: Vec<Span<'static>>,
        prefix_w: usize,
        text: String,
        text_style: Style,
    }
    let mut rows: Vec<Row> = Vec::new();

    let head_tag = "⬡ PLAN  ";
    rows.push(Row {
        prefix: vec![Span::styled(head_tag, styles::brand().bold())],
        prefix_w: head_tag.chars().count(),
        text: title.clone(),
        text_style: styles::brand().bold(),
    });

    for (i, step) in plan.steps.iter().enumerate() {
        let n = format!("{:>2} ", i + 1);
        rows.push(Row {
            prefix: vec![
                Span::styled(n.clone(), styles::muted().bold()),
                Span::styled("❯ ", styles::ok()),
            ],
            prefix_w: n.chars().count() + 2,
            text: step.title.trim().to_string(),
            text_style: Style::default(),
        });
        let d = step.detail.trim();
        if !d.is_empty() {
            rows.push(Row {
                prefix: vec![Span::raw("     ")],
                prefix_w: 5,
                text: d.to_string(),
                text_style: styles::muted(),
            });
        }
    }
    if let Some(notes) = plan
        .notes
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
    {
        rows.push(Row {
            prefix: vec![Span::styled("⚠ ", styles::warn())],
            prefix_w: 2,
            text: notes.to_string(),
            text_style: styles::warn(),
        });
    }

    let inner = rows
        .iter()
        .map(|r| r.prefix_w + r.text.chars().count())
        .max()
        .unwrap_or(20)
        .clamp(24, 72);
    let rule = |l: &str, r: &str| {
        TextLine::from(Span::styled(
            format!("  {l}{}{r}", "─".repeat(inner + 2)),
            crate::copy_mark::frame_row(frame),
        ))
    };

    let mut out = vec![rule("╭", "╮")];
    for (idx, row) in rows.iter().enumerate() {
        let avail = inner.saturating_sub(row.prefix_w).max(1);
        for (li, chunk) in wrap_words(&row.text, avail).iter().enumerate() {
            let mut spans = vec![Span::styled("  │ ", frame)];
            if li == 0 {
                spans.extend(row.prefix.iter().cloned());
            } else {
                spans.push(Span::raw(" ".repeat(row.prefix_w)));
            }
            let cw = chunk.chars().count();
            spans.push(Span::styled(chunk.clone(), row.text_style));
            let pad = inner.saturating_sub(row.prefix_w + cw);
            spans.push(Span::styled(format!("{} │", " ".repeat(pad)), frame));
            out.push(TextLine::from(spans));
        }
        if idx == 0 {
            out.push(rule("├", "┤")); // separate the title from the steps
        }
    }
    out.push(rule("╰", "╯"));
    out.push(TextLine::from(Span::styled(
        "    ▸ approve to build · or type your changes to revise",
        styles::muted(),
    )));
    out.push(TextLine::default());
    out
}
