//! The Forgeline colour tokens: one explicit value per role, per colour tier and mode.
//!
//! This is the only file (besides `degrade.rs`) that may contain colour literals. Renderers ask
//! for a role through [`crate::theme::t`] or [`crate::theme::styles`]; nothing is rounded
//! automatically, so every tier carries a hand-picked value.

use ratatui::style::{Color, Modifier, Style};

use super::{ColorTier, Mode};

/// One role: a foreground and/or background colour plus the modifiers that carry the meaning
/// when colour is unavailable (16-colour and mono tiers).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    pub fg: Color,
    pub bg: Color,
    pub mods: Modifier,
}

impl Token {
    const NONE: Self = Self {
        fg: Color::Reset,
        bg: Color::Reset,
        mods: Modifier::empty(),
    };

    const fn fg(color: Color) -> Self {
        Self {
            fg: color,
            ..Self::NONE
        }
    }

    const fn bg(color: Color) -> Self {
        Self {
            bg: color,
            ..Self::NONE
        }
    }

    const fn with(self, mods: Modifier) -> Self {
        Self {
            mods: self.mods.union(mods),
            ..self
        }
    }

    /// The token as a style: its colour(s) and modifiers.
    pub fn style(self) -> Style {
        let mut style = Style::default().add_modifier(self.mods);
        if self.fg != Color::Reset {
            style = style.fg(self.fg);
        }
        if self.bg != Color::Reset {
            style = style.bg(self.bg);
        }
        style
    }

    /// True when the token paints nothing and carries no modifier (plain terminal default).
    pub fn is_plain(self) -> bool {
        self == Self::NONE
    }
}

/// Every role in the Forgeline palette, resolved for one tier and mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub fg: Token,
    pub fg_muted: Token,
    pub fg_faint: Token,
    pub line: Token,
    pub brand: Token,
    pub brand_hot: Token,
    pub user: Token,
    pub ok: Token,
    pub warn: Token,
    pub err: Token,
    pub surface: Token,
    pub surface_raised: Token,
    pub diff_add_bg: Token,
    pub diff_del_bg: Token,
    pub diff_add_word: Token,
    pub diff_del_word: Token,
    pub cat: [Token; 6],
    pub heat: [Token; 5],
}

const BOLD: Modifier = Modifier::BOLD;
const DIM: Modifier = Modifier::DIM;

const fn rgb(hex: u32) -> Color {
    Color::Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

const fn ix(n: u8) -> Color {
    Color::Indexed(n)
}

impl Palette {
    pub fn resolve(tier: ColorTier, mode: Mode) -> Self {
        match (tier, mode) {
            (ColorTier::TrueColor, Mode::Dark) => TRUE_DARK,
            (ColorTier::TrueColor, Mode::Light) => TRUE_LIGHT,
            (ColorTier::Ansi256, Mode::Dark) => IDX_DARK,
            (ColorTier::Ansi256, Mode::Light) => IDX_LIGHT,
            (ColorTier::Ansi16, Mode::Dark) => ANSI_DARK,
            (ColorTier::Ansi16, Mode::Light) => ANSI_LIGHT,
            (ColorTier::Mono, _) => MONO,
        }
    }
}

const TRUE_DARK: Palette = Palette {
    fg: Token::fg(rgb(0xD8DCE8)),
    fg_muted: Token::fg(rgb(0x9096AA)),
    fg_faint: Token::fg(rgb(0x626779)),
    line: Token::fg(rgb(0x3A3C4A)),
    brand: Token::fg(rgb(0xFF8A30)),
    brand_hot: Token::fg(rgb(0xFFC247)),
    user: Token::fg(rgb(0x6FB1FF)),
    ok: Token::fg(rgb(0x5CD07A)),
    warn: Token::fg(rgb(0xE8B84A)),
    err: Token::fg(rgb(0xF26464)),
    surface: Token::bg(rgb(0x18181E)),
    surface_raised: Token::bg(rgb(0x25252E)),
    diff_add_bg: Token::bg(rgb(0x142B1C)),
    diff_del_bg: Token::bg(rgb(0x33181C)),
    diff_add_word: Token::bg(rgb(0x1F4A2D)),
    diff_del_word: Token::bg(rgb(0x5A2429)),
    cat: [
        Token::fg(rgb(0xFF8A30)),
        Token::fg(rgb(0x6FB1FF)),
        Token::fg(rgb(0x4FC9A4)),
        Token::fg(rgb(0xB08CFF)),
        Token::fg(rgb(0xF27BA8)),
        Token::fg(rgb(0xD9C27A)),
    ],
    heat: [
        Token::fg(rgb(0xD82D1C)),
        Token::fg(rgb(0xFF6020)),
        Token::fg(rgb(0xFF8A30)),
        Token::fg(rgb(0xFFC247)),
        Token::fg(rgb(0xFFEE96)),
    ],
};

const TRUE_LIGHT: Palette = Palette {
    fg: Token::fg(rgb(0x1D2030)),
    fg_muted: Token::fg(rgb(0x565B6B)),
    fg_faint: Token::fg(rgb(0x8B8D98)),
    line: Token::fg(rgb(0xD3D0C8)),
    brand: Token::fg(rgb(0xB34E09)),
    brand_hot: Token::fg(rgb(0xB86A00)),
    user: Token::fg(rgb(0x1A66C7)),
    ok: Token::fg(rgb(0x17793A)),
    warn: Token::fg(rgb(0x8F6200)),
    err: Token::fg(rgb(0xC7312F)),
    surface: Token::bg(rgb(0xF3F1EC)),
    surface_raised: Token::bg(rgb(0xE7E3DA)),
    diff_add_bg: Token::bg(rgb(0xE0F4E6)),
    diff_del_bg: Token::bg(rgb(0xFBE1E1)),
    diff_add_word: Token::bg(rgb(0xBFE8CB)),
    diff_del_word: Token::bg(rgb(0xF3BDBD)),
    cat: [
        Token::fg(rgb(0xB34E09)),
        Token::fg(rgb(0x1A66C7)),
        Token::fg(rgb(0x138A6E)),
        Token::fg(rgb(0x7552D6)),
        Token::fg(rgb(0xC23B7A)),
        Token::fg(rgb(0x8E7420)),
    ],
    heat: [
        Token::fg(rgb(0x9E2A12)),
        Token::fg(rgb(0xB8420A)),
        Token::fg(rgb(0xB34E09)),
        Token::fg(rgb(0xB86A00)),
        Token::fg(rgb(0x8F6200)),
    ],
};

const IDX_DARK: Palette = Palette {
    fg: Token::fg(ix(253)),
    fg_muted: Token::fg(ix(246)),
    fg_faint: Token::fg(ix(242)),
    line: Token::fg(ix(238)),
    brand: Token::fg(ix(208)),
    brand_hot: Token::fg(ix(214)),
    user: Token::fg(ix(75)),
    ok: Token::fg(ix(78)),
    warn: Token::fg(ix(179)),
    err: Token::fg(ix(203)),
    surface: Token::bg(ix(234)),
    surface_raised: Token::bg(ix(236)),
    diff_add_bg: Token::bg(ix(22)),
    diff_del_bg: Token::bg(ix(52)),
    diff_add_word: Token::bg(ix(28)),
    diff_del_word: Token::bg(ix(88)),
    cat: [
        Token::fg(ix(208)),
        Token::fg(ix(75)),
        Token::fg(ix(79)),
        Token::fg(ix(141)),
        Token::fg(ix(211)),
        Token::fg(ix(186)),
    ],
    heat: [
        Token::fg(ix(160)),
        Token::fg(ix(202)),
        Token::fg(ix(208)),
        Token::fg(ix(214)),
        Token::fg(ix(229)),
    ],
};

const IDX_LIGHT: Palette = Palette {
    fg: Token::fg(ix(235)),
    fg_muted: Token::fg(ix(240)),
    fg_faint: Token::fg(ix(245)),
    line: Token::fg(ix(252)),
    brand: Token::fg(ix(130)),
    brand_hot: Token::fg(ix(136)),
    user: Token::fg(ix(26)),
    ok: Token::fg(ix(28)),
    warn: Token::fg(ix(94)),
    err: Token::fg(ix(160)),
    surface: Token::bg(ix(255)),
    surface_raised: Token::bg(ix(254)),
    diff_add_bg: Token::bg(ix(194)),
    diff_del_bg: Token::bg(ix(224)),
    diff_add_word: Token::bg(ix(157)),
    diff_del_word: Token::bg(ix(217)),
    cat: [
        Token::fg(ix(130)),
        Token::fg(ix(26)),
        Token::fg(ix(30)),
        Token::fg(ix(98)),
        Token::fg(ix(162)),
        Token::fg(ix(136)),
    ],
    heat: [
        Token::fg(ix(124)),
        Token::fg(ix(130)),
        Token::fg(ix(130)),
        Token::fg(ix(136)),
        Token::fg(ix(94)),
    ],
};

const ANSI_DARK: Palette = Palette {
    fg: Token::NONE,
    fg_muted: Token::NONE.with(DIM),
    fg_faint: Token::fg(Color::DarkGray),
    line: Token::fg(Color::DarkGray),
    brand: Token::fg(Color::Yellow),
    brand_hot: Token::fg(Color::LightYellow),
    user: Token::fg(Color::Blue),
    ok: Token::fg(Color::Green),
    warn: Token::fg(Color::LightYellow).with(BOLD),
    err: Token::fg(Color::Red),
    surface: Token::NONE,
    surface_raised: Token::NONE,
    diff_add_bg: Token::NONE,
    diff_del_bg: Token::NONE,
    diff_add_word: Token::fg(Color::Green).with(BOLD),
    diff_del_word: Token::fg(Color::Red).with(BOLD),
    cat: [
        Token::fg(Color::Yellow),
        Token::fg(Color::Blue),
        Token::fg(Color::Cyan),
        Token::fg(Color::Magenta),
        Token::fg(Color::LightRed),
        Token::fg(Color::Green),
    ],
    heat: [
        Token::fg(Color::Red),
        Token::fg(Color::Red),
        Token::fg(Color::Yellow),
        Token::fg(Color::LightYellow),
        Token::fg(Color::White),
    ],
};

const ANSI_LIGHT: Palette = Palette {
    brand: Token::fg(Color::Yellow).with(BOLD),
    brand_hot: Token::fg(Color::Yellow),
    warn: Token::fg(Color::Yellow).with(BOLD),
    cat: [
        Token::fg(Color::Yellow),
        Token::fg(Color::Blue),
        Token::fg(Color::Cyan),
        Token::fg(Color::Magenta),
        Token::fg(Color::Red),
        Token::fg(Color::Green),
    ],
    heat: [
        Token::fg(Color::Red),
        Token::fg(Color::Red),
        Token::fg(Color::Yellow),
        Token::fg(Color::Yellow),
        Token::fg(Color::Yellow),
    ],
    ..ANSI_DARK
};

const MONO: Palette = Palette {
    fg: Token::NONE,
    fg_muted: Token::NONE.with(DIM),
    fg_faint: Token::NONE.with(DIM),
    line: Token::NONE,
    brand: Token::NONE.with(BOLD),
    brand_hot: Token::NONE.with(BOLD),
    user: Token::NONE,
    ok: Token::NONE,
    warn: Token::NONE.with(BOLD),
    err: Token::NONE.with(BOLD),
    surface: Token::NONE,
    surface_raised: Token::NONE,
    diff_add_bg: Token::NONE,
    diff_del_bg: Token::NONE,
    diff_add_word: Token::NONE.with(BOLD),
    diff_del_word: Token::NONE.with(BOLD),
    cat: [Token::NONE; 6],
    heat: [Token::NONE; 5],
};
