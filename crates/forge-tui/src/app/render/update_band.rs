//! The update card, above the composer: a newer Forge, what changed, and the keys that act on it.
//!
//! It is a live-region band like the queued follow-ups: never scrollback, and gone the moment the
//! user decides. The state lives in [`crate::update_card::UpdateCard`].

use super::list_view::fit;
use super::*;
use crate::theme::glyphs::g;
use crate::theme::styles;
use crate::update_card::{CardPhase, ServiceNote, UpdatePlan};

/// Release-note rows shown at most; a longer note is read from the release page.
const MAX_NOTES: usize = 3;

/// The card as lines for `width` columns, keeping the title and the keys first when `room` is
/// short and dropping the notes before anything else.
pub(crate) fn lines(app: &App, width: u16, room: u16) -> Vec<TextLine<'static>> {
    let card = &app.update_card;
    if !card.visible(app.busy) {
        return Vec::new();
    }
    let Some(offer) = card.offer_ref() else {
        return Vec::new();
    };
    let width = width as usize;
    let glyphs = g();
    let row = |text: String, style: Style| TextLine::from(Span::styled(fit(&text, width), style));
    let mut title_and_keys: Vec<TextLine<'static>> = Vec::new();
    let mut detail: Vec<TextLine<'static>> = Vec::new();
    match card.phase() {
        CardPhase::Hidden => return Vec::new(),
        CardPhase::Offer => {
            title_and_keys.push(row(
                format!(
                    "{} Forge {} is available (you have {})",
                    glyphs.brand, offer.version, offer.current
                ),
                styles::warn().add_modifier(Modifier::BOLD),
            ));
            for note in offer.notes.iter().take(MAX_NOTES) {
                detail.push(row(format!("  - {note}"), styles::muted()));
            }
            if let UpdatePlan::Command(cmd) = &offer.plan {
                detail.push(row(format!("  run: {cmd}"), styles::muted()));
            }
            detail.push(row(format!("  {}", offer.url), styles::faint()));
            if app.input.is_empty() {
                let go = match offer.plan {
                    UpdatePlan::InPlace => "Update now",
                    UpdatePlan::Command(_) => "Run it now",
                };
                let mut keys = vec![("u", go)];
                if matches!(offer.plan, UpdatePlan::Command(_)) {
                    keys.push(("c", "Copy command"));
                }
                keys.extend([("l", "Later"), ("s", "Skip this version")]);
                title_and_keys.push(TextLine::from(
                    [Span::raw("  ")]
                        .into_iter()
                        .chain(styles::hint_row(&keys))
                        .collect::<Vec<_>>(),
                ));
            } else {
                title_and_keys.push(row(
                    "  finish or clear your message, then press u / l / s".to_string(),
                    styles::faint(),
                ));
            }
        }
        CardPhase::Running(status) => {
            let spin = glyphs.spinner[app.tick % glyphs.spinner.len()];
            title_and_keys.push(row(
                format!(
                    "{spin} Updating Forge to {} {} {status}",
                    offer.version, glyphs.sep
                ),
                styles::warn().add_modifier(Modifier::BOLD),
            ));
        }
        CardPhase::Done => {
            title_and_keys.push(row(
                format!(
                    "{} Updated to {} {} restart Forge to use it",
                    glyphs.ok, offer.version, glyphs.sep
                ),
                styles::ok().add_modifier(Modifier::BOLD),
            ));
            let mut keys = vec![("r", "Restart now")];
            if card.service() == &ServiceNote::Running {
                keys.push(("v", "Restart the Forge service"));
            }
            keys.push(("l", "Later"));
            let mut spans = vec![Span::raw("  ")];
            spans.extend(styles::hint_row(&keys));
            title_and_keys.push(TextLine::from(spans));
            match card.service() {
                ServiceNote::None | ServiceNote::Running => {}
                ServiceNote::Restarting => {
                    detail.push(row(
                        "  restarting the Forge service…".into(),
                        styles::muted(),
                    ));
                }
                ServiceNote::Restarted => {
                    detail.push(row("  the Forge service restarted".into(), styles::muted()));
                }
                ServiceNote::Held(why) => {
                    detail.push(row(
                        format!("  service not restarted: {why}"),
                        styles::warn(),
                    ));
                }
            }
            if card.service() == &ServiceNote::Running {
                detail.push(row(
                    "  the Forge service is still running the old version".into(),
                    styles::muted(),
                ));
            }
        }
        CardPhase::Failed(why) => {
            title_and_keys.push(row(
                format!(
                    "{} Update failed: {}",
                    glyphs.fail,
                    why.lines().next().unwrap_or("")
                ),
                styles::err().add_modifier(Modifier::BOLD),
            ));
            let mut spans = vec![Span::raw("  ")];
            spans.extend(styles::hint_row(&[("u", "Retry"), ("l", "Dismiss")]));
            title_and_keys.push(TextLine::from(spans));
            detail.push(row(
                format!("  or update by hand: {}", offer.url),
                styles::faint(),
            ));
        }
    }
    let room = room as usize;
    let mut out = Vec::new();
    let spare = room.saturating_sub(title_and_keys.len());
    let mut title_and_keys = title_and_keys.into_iter();
    out.extend(title_and_keys.next());
    out.extend(detail.into_iter().take(spare));
    out.extend(title_and_keys);
    // The keys are the point of the card: if even they do not fit, the title alone is kept.
    out.truncate(room.max(1));
    out
}

/// Rows the card takes, given the `avail` rows the layout can spare.
pub(crate) fn height(app: &App, width: u16, avail: u16) -> u16 {
    (lines(app, width, avail).len() as u16).min(avail)
}

pub(crate) fn render(frame: &mut Frame, area: Rect, app: &App) {
    if area.height == 0 {
        return;
    }
    frame.render_widget(Paragraph::new(lines(app, area.width, area.height)), area);
}
