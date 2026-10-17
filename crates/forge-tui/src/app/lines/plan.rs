//! Scrollback line builders: the plan card.

use crate::theme::glyphs::g;
use crate::theme::styles;
use ratatui::style::Style;
use ratatui::text::{Line as TextLine, Span};

use crate::copy_mark;

/// Render a proposed plan (the `present_plan` tool) as a titled block: a `PLAN` label with the
/// title and step count, numbered steps whose wrapped text hangs under the step title, muted
/// per-step detail, muted risks, and a faint hint for the approve prompt that follows. Printed
/// in full in both display modes so the steps a question refers to are always on screen.
pub(crate) fn plan_card_lines(plan: &forge_types::PlanProposal) -> Vec<TextLine<'static>> {
    let glyphs = g();
    let title = plan.title.trim();
    let steps = plan.steps.len();
    let mut out = vec![TextLine::from(vec![
        Span::styled("  PLAN", styles::brand().bold()),
        Span::styled(format!("  {title}"), styles::fg().bold()),
        Span::styled(
            format!(
                "  {} {steps} step{}",
                glyphs.sep,
                if steps == 1 { "" } else { "s" }
            ),
            styles::faint(),
        ),
    ])];
    let number_w = steps.to_string().len().max(1);
    let hang = 2 + number_w + 2;
    for (i, step) in plan.steps.iter().enumerate() {
        let number = format!("  {:>number_w$}  ", i + 1);
        push_wrapped(
            &mut out,
            Span::styled(number, styles::muted().bold()),
            hang,
            step.title.trim(),
            Style::default(),
        );
        let detail = step.detail.trim();
        if !detail.is_empty() {
            push_wrapped(
                &mut out,
                Span::raw(" ".repeat(hang)),
                hang,
                detail,
                styles::muted(),
            );
        }
    }
    if let Some(notes) = plan
        .notes
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
    {
        push_wrapped(
            &mut out,
            Span::styled("  Risks: ", styles::muted().bold()),
            "  Risks: ".len(),
            notes,
            styles::muted(),
        );
    }
    out.push(TextLine::from(Span::styled(
        "  approve to build · or type your changes to revise",
        styles::faint(),
    )));
    out.push(TextLine::default());
    out
}

/// A row that starts with `lead`. The wrap pass breaks it at the live width and hangs the
/// continuation rows at the end of `lead`, so a narrow terminal never leaves a flush-left orphan.
fn push_wrapped(
    out: &mut Vec<TextLine<'static>>,
    lead: Span<'static>,
    _hang: usize,
    text: &str,
    style: Style,
) {
    let lead = Span::styled(lead.content, copy_mark::hang(lead.style));
    out.push(TextLine::from(vec![
        Span::styled("", copy_mark::prose(Style::default())),
        lead,
        Span::styled(text.to_string(), style),
    ]));
}
