//! One meaning per glyph, with an ASCII twin for every one (Linux console, `TERM=dumb`,
//! non-UTF-8 and CJK locales). Everything in the Unicode set is single-width.

use ratatui::symbols::border;

use super::{t, GlyphTier};

/// The glyph set for one tier.
#[derive(Debug)]
pub struct Glyphs {
    /// Forge / assistant identity, brand mark.
    pub brand: &'static str,
    pub rail_user: &'static str,
    pub rail_assistant: &'static str,
    /// Decision rail (permission, question).
    pub rail_decision: &'static str,
    /// Composer prompt and list focus caret.
    pub prompt: &'static str,
    pub disclose_closed: &'static str,
    pub disclose_open: &'static str,
    pub card_gutter: &'static str,
    pub spinner: &'static [&'static str],
    /// Static spinner stand-in under reduced motion.
    pub spinner_still: &'static str,
    pub awaiting: &'static str,
    pub ok: &'static str,
    pub fail: &'static str,
    pub warn: &'static str,
    pub info: &'static str,
    pub group: &'static str,
    pub live_dot: &'static str,
    pub task_open: &'static str,
    pub task_active: &'static str,
    pub task_done: &'static str,
    pub gauge_full: &'static str,
    pub gauge_empty: &'static str,
    pub branch: &'static str,
    pub goal: &'static str,
    pub loop_: &'static str,
    pub sep: &'static str,
    pub ellipsis: &'static str,
    pub up_down: &'static str,
    pub left_right: &'static str,
    pub up: &'static str,
    pub down: &'static str,
    /// Token direction markers in prose (`↑72 ↓30`, or `up 72 down 30` in ASCII).
    pub tok_up: &'static str,
    pub tok_down: &'static str,
    pub diff_minus: &'static str,
    pub caret: &'static str,
    pub border: border::Set<'static>,
    pub unicode: bool,
}

const UNICODE: Glyphs = Glyphs {
    brand: "✦",
    rail_user: "▎",
    rail_assistant: "▏",
    rail_decision: "┃",
    prompt: "❯",
    disclose_closed: "▸",
    disclose_open: "▾",
    card_gutter: "┊",
    spinner: &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"],
    spinner_still: "…",
    awaiting: "◇",
    ok: "✓",
    fail: "✗",
    warn: "▲",
    info: "·",
    group: "◆",
    live_dot: "●",
    task_open: "○",
    task_active: "◉",
    task_done: "✓",
    gauge_full: "▰",
    gauge_empty: "▱",
    branch: "⑂",
    goal: "◎",
    loop_: "↻",
    sep: "·",
    ellipsis: "…",
    up_down: "↑↓",
    left_right: "←→",
    up: "↑",
    down: "↓",
    tok_up: "↑",
    tok_down: "↓",
    diff_minus: "−",
    caret: "▍",
    border: border::ROUNDED,
    unicode: true,
};

const ASCII: Glyphs = Glyphs {
    brand: "*",
    rail_user: "|",
    rail_assistant: ":",
    rail_decision: "#",
    prompt: ">",
    disclose_closed: "+",
    disclose_open: "-",
    card_gutter: ":",
    spinner: &["|", "/", "-", "\\"],
    spinner_still: "...",
    awaiting: "o",
    ok: "+",
    fail: "x",
    warn: "!",
    info: "-",
    group: "*",
    live_dot: "*",
    task_open: "o",
    task_active: "@",
    task_done: "x",
    gauge_full: "#",
    gauge_empty: "-",
    branch: "git:",
    goal: "goal",
    loop_: "loop",
    sep: "-",
    ellipsis: "...",
    up_down: "^v",
    left_right: "<>",
    up: "^",
    down: "v",
    tok_up: "up ",
    tok_down: "down ",
    diff_minus: "-",
    caret: "_",
    border: border::Set {
        top_left: "+",
        top_right: "+",
        bottom_left: "+",
        bottom_right: "+",
        vertical_left: "|",
        vertical_right: "|",
        horizontal_top: "-",
        horizontal_bottom: "-",
    },
    unicode: false,
};

pub fn set(tier: GlyphTier) -> &'static Glyphs {
    match tier {
        GlyphTier::Unicode => &UNICODE,
        GlyphTier::Ascii => &ASCII,
    }
}

/// The glyph set of the active theme.
pub fn g() -> &'static Glyphs {
    t().glyphs()
}

impl Glyphs {
    /// The spinner frame for `tick`, or the static stand-in under reduced motion.
    pub fn spin(&self, tick: usize) -> &'static str {
        if t().animates() {
            self.spinner[tick % self.spinner.len()]
        } else {
            self.spinner_still
        }
    }

    /// A `▰▰▱▱` bar of `cells` cells filled to `frac` (0.0..=1.0).
    pub fn bar(&self, frac: f64, cells: usize) -> String {
        let filled = ((frac.clamp(0.0, 1.0) * cells as f64).round() as usize).min(cells);
        format!(
            "{}{}",
            self.gauge_full.repeat(filled),
            self.gauge_empty.repeat(cells - filled)
        )
    }
}

/// Re-glyph text that carries retired glyphs (`⚠ ✖ ⚒ …`) onto the active glyph set, then, in the
/// ASCII tier, fold every remaining non-ASCII character to a readable ASCII twin. Used for text
/// that arrives pre-glyphed (command notes, the headless presenter) so one glyph meaning holds
/// everywhere and a Linux console or `TERM=dumb` terminal never sees an unprintable character.
pub fn normalise(text: &str) -> String {
    let glyphs = g();
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '⚠' => out.push_str(glyphs.warn),
            '✖' | '✕' => out.push_str(glyphs.fail),
            '⚒' => out.push_str(glyphs.brand),
            '⚙' => out.push_str(glyphs.group),
            '※' => out.push_str(glyphs.info),
            '❓' => out.push('?'),
            c if glyphs.unicode || c.is_ascii() => out.push(c),
            c => out.push_str(ascii_twin(c)),
        }
    }
    out
}

fn ascii_twin(c: char) -> &'static str {
    match c {
        '●' | '◆' | '✦' | '⬡' | '⛓' | '⚖' | '⌬' | '◈' | '▰' => "*",
        '○' | '◇' => "o",
        '◉' => "@",
        '✓' | '▸' => "+",
        '✗' | '▾' => "-",
        '▲' => "!",
        '·' | '—' | '–' => "-",
        '…' | '⋯' => "...",
        '→' => "->",
        '←' => "<-",
        '↑' => "up ",
        '↓' => "down ",
        '↳' | '⤷' | '▶' | '❯' | '›' => ">",
        '▎' | '┃' | '│' => "|",
        '▏' | '┊' => ":",
        '─' | '━' | '╌' | '┄' => "-",
        '╭' | '╮' | '╰' | '╯' | '├' | '┤' | '┌' | '┐' | '└' | '┘' => "+",
        '█' => "#",
        '░' | '▱' => "-",
        '⟳' => "~",
        '⇧' => "^",
        '⏎' => "Enter",
        '⇥' => "Tab",
        'Σ' => "total",
        '⑂' => "git:",
        '“' | '”' => "\"",
        '‘' | '’' => "'",
        _ => "?",
    }
}
