//! The faint two-row record an answered `ask_user` question leaves in scrollback: what was asked,
//! then what was chosen, so the decision stays readable without competing with the turn around it.

use super::rail;
use crate::copy_mark;
use crate::theme::glyphs::g;
use crate::theme::styles::{self, RailKind};
use forge_types::{Answer, Question};
use ratatui::style::Style;
use ratatui::text::{Line, Span};

fn question_line(q: &Question) -> String {
    let header = q.header.trim();
    if header.is_empty() {
        q.text.clone()
    } else {
        format!("{header} {} {}", g().sep, q.text)
    }
}

/// The description of the one option the answer picked, when it picked exactly one.
fn chosen_description<'a>(q: &'a Question, answer: &Answer) -> Option<&'a str> {
    let [label] = answer.selected.as_slice() else {
        return None;
    };
    q.options
        .iter()
        .find(|o| &o.label == label)
        .map(|o| o.description.trim())
        .filter(|d| !d.is_empty())
}

pub(crate) fn decision_rows(q: &Question, answer: Option<&Answer>) -> Vec<Line<'static>> {
    let asked = rail::text_row(RailKind::User, &question_line(q), styles::faint());
    let mut spans = rail::rail_spans(RailKind::User);
    spans.push(Span::styled(
        String::new(),
        copy_mark::prose(Style::default()),
    ));
    match answer {
        Some(a) if !a.is_empty() => {
            spans.push(Span::styled(format!("{} ", g().ok), styles::ok()));
            // A secret never lands in scrollback, not even as a record of what was typed.
            let shown = if q.secret {
                "hidden".to_string()
            } else {
                a.summary()
            };
            spans.push(Span::styled(shown, styles::ok().bold()));
            if let Some(d) = chosen_description(q, a) {
                spans.push(Span::styled(format!("  {d}"), styles::faint()));
            }
        }
        Some(_) => spans.push(Span::styled("(no answer)", styles::muted())),
        None => spans.push(Span::styled("(dismissed)", styles::muted())),
    }
    vec![asked, Line::from(spans)]
}
