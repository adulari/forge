//! Nearest-colour mapping for colours that did not come from a token (syntect output, external
//! statusline SGR, legacy literals). Tokens carry hand-picked values per tier and never go through
//! this code; it is the safety net used by the remapping backend.

use ratatui::style::Color;

use super::ColorTier;

const ANSI16: [(Color, [u8; 3]); 16] = [
    (Color::Black, [0, 0, 0]),
    (Color::Red, [205, 0, 0]),
    (Color::Green, [0, 205, 0]),
    (Color::Yellow, [205, 205, 0]),
    (Color::Blue, [0, 0, 238]),
    (Color::Magenta, [205, 0, 205]),
    (Color::Cyan, [0, 205, 205]),
    (Color::Gray, [229, 229, 229]),
    (Color::DarkGray, [127, 127, 127]),
    (Color::LightRed, [255, 0, 0]),
    (Color::LightGreen, [0, 255, 0]),
    (Color::LightYellow, [255, 255, 0]),
    (Color::LightBlue, [92, 92, 255]),
    (Color::LightMagenta, [255, 0, 255]),
    (Color::LightCyan, [0, 255, 255]),
    (Color::White, [255, 255, 255]),
];

/// The RGB value of an xterm-256 index (indices below 16 use the default xterm palette).
pub fn indexed_rgb(n: u8) -> [u8; 3] {
    match n {
        0..=15 => ANSI16[n as usize].1,
        16..=231 => {
            let i = n - 16;
            let level = |c: u8| if c == 0 { 0 } else { 55 + 40 * c };
            [level(i / 36), level(i / 6 % 6), level(i % 6)]
        }
        232..=255 => {
            let v = 8 + 10 * (n - 232);
            [v, v, v]
        }
    }
}

fn lab(rgb: [u8; 3]) -> [f32; 3] {
    let lin = |c: u8| {
        let c = f32::from(c) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let (r, g, b) = (lin(rgb[0]), lin(rgb[1]), lin(rgb[2]));
    let x = (0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.95047;
    let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    let z = (0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.08883;
    let f = |t: f32| {
        if t > 0.008856 {
            t.cbrt()
        } else {
            7.787 * t + 16.0 / 116.0
        }
    };
    let (fx, fy, fz) = (f(x), f(y), f(z));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)
}

/// Nearest index in the 6x6x6 cube and grey ramp (16..=255) by CIE76 distance.
pub fn nearest_256(rgb: [u8; 3]) -> u8 {
    let target = lab(rgb);
    (16u8..=255)
        .min_by(|a, b| {
            distance(lab(indexed_rgb(*a)), target)
                .total_cmp(&distance(lab(indexed_rgb(*b)), target))
        })
        .unwrap_or(16)
}

/// Nearest of the 16 ANSI colours by CIE76 distance.
pub fn nearest_16(rgb: [u8; 3]) -> Color {
    let target = lab(rgb);
    ANSI16
        .iter()
        .min_by(|a, b| distance(lab(a.1), target).total_cmp(&distance(lab(b.1), target)))
        .map(|(c, _)| *c)
        .unwrap_or(Color::Reset)
}

pub(crate) fn color_rgb(color: Color) -> Option<[u8; 3]> {
    match color {
        Color::Rgb(r, g, b) => Some([r, g, b]),
        Color::Indexed(n) => Some(indexed_rgb(n)),
        _ => None,
    }
}

/// Map any colour into what `tier` can show. Reset and named colours are already tier-safe
/// except in Mono, where everything becomes Reset.
pub fn map_color(color: Color, tier: ColorTier) -> Color {
    match tier {
        ColorTier::TrueColor => color,
        ColorTier::Mono => Color::Reset,
        ColorTier::Ansi256 => match color {
            Color::Rgb(r, g, b) => Color::Indexed(nearest_256([r, g, b])),
            other => other,
        },
        ColorTier::Ansi16 => match color_rgb(color) {
            Some(rgb) => nearest_16(rgb),
            None => color,
        },
    }
}

/// The foreground SGR parameters for `color`, or `None` for Reset.
pub fn sgr_fg(color: Color) -> Option<String> {
    let named = |base: u8, i: u8| Some(format!("{}", base + i));
    match color {
        Color::Reset => None,
        Color::Black => named(30, 0),
        Color::Red => named(30, 1),
        Color::Green => named(30, 2),
        Color::Yellow => named(30, 3),
        Color::Blue => named(30, 4),
        Color::Magenta => named(30, 5),
        Color::Cyan => named(30, 6),
        Color::Gray => named(30, 7),
        Color::DarkGray => named(90, 0),
        Color::LightRed => named(90, 1),
        Color::LightGreen => named(90, 2),
        Color::LightYellow => named(90, 3),
        Color::LightBlue => named(90, 4),
        Color::LightMagenta => named(90, 5),
        Color::LightCyan => named(90, 6),
        Color::White => named(90, 7),
        Color::Indexed(n) => Some(format!("38;5;{n}")),
        Color::Rgb(r, g, b) => Some(format!("38;2;{r};{g};{b}")),
    }
}
