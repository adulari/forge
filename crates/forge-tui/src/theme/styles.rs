//! Role styles derived from the palette: what renderers call instead of picking colours.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

use super::glyphs::g;
use super::t;
use super::tokens::Token;
use crate::surface::SurfaceTone;

pub fn fg() -> Style {
    t().palette.fg.style()
}
pub fn muted() -> Style {
    t().palette.fg_muted.style()
}
pub fn faint() -> Style {
    t().palette.fg_faint.style()
}
pub fn line() -> Style {
    t().palette.line.style()
}
pub fn brand() -> Style {
    t().palette.brand.style()
}
pub fn brand_hot() -> Style {
    t().palette.brand_hot.style()
}
pub fn user() -> Style {
    t().palette.user.style()
}
pub fn ok() -> Style {
    t().palette.ok.style()
}
pub fn warn() -> Style {
    t().palette.warn.style()
}
pub fn err() -> Style {
    t().palette.err.style()
}
pub fn surface() -> Style {
    t().palette.surface.style()
}

/// Categorical series colour (board lanes, mesh providers, agent identities), wrapping at six.
pub fn category(i: usize) -> Style {
    t().palette.cat[i % 6].style()
}

/// Ember heat ramp, `step` 0 (cool) to 4 (white-hot). Static in Mono and under reduced motion
/// callers pick their own step.
pub fn heat(step: usize) -> Style {
    t().palette.heat[step.min(4)].style()
}

/// A key in a hint: bold body text.
pub fn key_chip() -> Style {
    t().palette.fg.style().add_modifier(Modifier::BOLD)
}

/// One hint: `key` as a chip, then its label in faint.
pub fn hint_spans(key: &str, label: &str) -> [Span<'static>; 2] {
    [
        Span::styled(key.to_string(), key_chip()),
        Span::styled(format!(" {label}"), faint()),
    ]
}

/// A hint row from `(key, label)` pairs, separated by a faint `·`.
pub fn hint_row(pairs: &[(&str, &str)]) -> Vec<Span<'static>> {
    let sep = format!(" {} ", g().sep);
    let mut out = Vec::new();
    for (i, (key, label)) in pairs.iter().enumerate() {
        if i > 0 {
            out.push(Span::styled(sep.clone(), faint()));
        }
        out.extend(hint_spans(key, label));
    }
    out
}

/// A selected list row: raised fill where backgrounds are painted, bold everywhere. The caller
/// draws the `❯` caret, so 16-colour and mono selection never depends on a fill alone.
pub fn selected_row() -> Style {
    t().palette
        .surface_raised
        .style()
        .add_modifier(Modifier::BOLD)
}

/// The two colours a turn rail takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RailKind {
    User,
    /// The assistant while streaming.
    Live,
    /// A settled assistant rail, `stage` 0 (just finished) to 2 (fully cooled).
    Settled(u8),
    /// Permission and question blocks.
    Decision,
    Error,
}

pub fn rail(kind: RailKind) -> Style {
    let p = t().palette;
    match kind {
        RailKind::User => p.user.style(),
        RailKind::Live => p.brand.style(),
        RailKind::Settled(0) if t().animates() => p.brand.style(),
        RailKind::Settled(1) if t().animates() => p.fg_faint.style(),
        RailKind::Settled(_) => p.line.style(),
        RailKind::Decision => p.warn.style(),
        RailKind::Error => p.err.style(),
    }
}

/// The four notice kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeKind {
    Ok,
    Info,
    Warn,
    Err,
}

/// A notice's glyph and style.
pub fn notice(kind: NoticeKind) -> (&'static str, Style) {
    let glyphs = g();
    match kind {
        NoticeKind::Ok => (glyphs.ok, ok()),
        NoticeKind::Info => (glyphs.info, muted()),
        NoticeKind::Warn => (glyphs.warn, warn()),
        NoticeKind::Err => (glyphs.fail, err()),
    }
}

/// The mode (temper) style by permissiveness: Read-only user, Ask muted, Auto-edit brand, Auto
/// warn, Full err and bold. Unknown labels read as muted.
pub fn temper(label: &str) -> Style {
    let p = t().palette;
    match label {
        "Read-only" => p.user.style(),
        "Ask" => p.fg_muted.style(),
        "Auto-edit" => p.brand.style(),
        "Auto" => p.warn.style(),
        "Full" => p.err.style().add_modifier(Modifier::BOLD),
        _ => p.fg_muted.style(),
    }
}

/// The foreground colour of [`temper`] (for callers that only take a colour).
pub fn temper_color(label: &str) -> Color {
    temper(label).fg.unwrap_or(Color::Reset)
}

/// Effort rung style: low faint, medium muted, high brand, xhigh brand_hot, WHITE·HOT animates
/// over the heat ramp (static brand_hot and bold under reduced motion).
pub fn effort(level: forge_types::EffortLevel, tick: usize) -> Style {
    use forge_types::EffortLevel as E;
    let p = t().palette;
    match level {
        E::Low => p.fg_faint.style(),
        E::Medium => p.fg_muted.style(),
        E::High => p.brand.style(),
        E::XHigh => p.brand_hot.style(),
        E::WhiteHot => {
            if t().animates() {
                p.heat[(tick / 3) % 5].style().add_modifier(Modifier::BOLD)
            } else {
                p.brand_hot.style().add_modifier(Modifier::BOLD)
            }
        }
    }
}

/// Context gauge colour: muted below 70 percent, warn from 70, err from 90.
pub fn gauge(frac: f64) -> Style {
    if frac >= 0.9 {
        err()
    } else if frac >= 0.7 {
        warn()
    } else {
        muted()
    }
}

/// Spinner colour: heat1 to heat3 and back over about 1.2 s; two colours in the 16-colour tier,
/// static brand under reduced motion.
pub fn spinner(tick: usize) -> Style {
    let p = t().palette;
    if !t().animates() {
        return p.brand.style();
    }
    let phase = tick % 16;
    let step = if phase < 8 { phase } else { 16 - phase };
    let token = match t().tier {
        super::ColorTier::Ansi16 => {
            if step < 4 {
                p.heat[1]
            } else {
                p.heat[3]
            }
        }
        _ => p.heat[1 + step * 2 / 8],
    };
    token.style()
}

/// Border style for a frame: the neutral line, or the tone colour when focused or stateful.
pub(crate) fn frame(tone: SurfaceTone, focused: bool) -> Style {
    if focused {
        tone_token(tone).style()
    } else {
        t().palette.line.style()
    }
}

pub(crate) fn tone_token(tone: SurfaceTone) -> Token {
    let p = t().palette;
    match tone {
        SurfaceTone::Brand => p.brand,
        SurfaceTone::Accent => p.user,
        SurfaceTone::Tool => p.fg_muted,
        SurfaceTone::Success => p.ok,
        SurfaceTone::Warning => p.warn,
        SurfaceTone::Danger => p.err,
    }
}

/// Row colour for a model/pin picker row by availability: ok, warn, err or faint.
pub fn availability(ok_: bool, benched: bool, paid: bool) -> Style {
    if benched {
        faint()
    } else if !ok_ {
        err()
    } else if paid {
        warn()
    } else {
        ok()
    }
}

/// The style of a mouse-selected cell: a blue fill where colours exist, REVERSED where they do
/// not (mono), so a selection is always visible.
pub fn selection() -> Style {
    if t().tier == super::ColorTier::Mono {
        Style::default().add_modifier(Modifier::REVERSED)
    } else {
        Style::default().bg(super::color::selection())
    }
}

/// A solid pill in a role colour (floating bars, labels): dark text on the fill where fills are
/// painted, REVERSED role text where they are not (16-colour and mono).
pub fn pill(role: Style) -> Style {
    if t().paints_backgrounds() {
        Style::default()
            .fg(super::color::on_fill())
            .bg(role.fg.unwrap_or(Color::Reset))
            .add_modifier(Modifier::BOLD)
    } else {
        role.add_modifier(Modifier::REVERSED | Modifier::BOLD)
    }
}

/// The input cursor block: a brand fill with dark text, or REVERSED where fills are not painted.
pub fn cursor() -> Style {
    if t().paints_backgrounds() {
        Style::default()
            .fg(super::color::on_fill())
            .bg(super::color::brand())
    } else {
        Style::default().add_modifier(Modifier::REVERSED)
    }
}
