//! Named accent palettes and per-role overrides for truecolor terminals. The base palette from
//! [`super::tokens`] still supplies surfaces, text and diff colours; a named palette swaps only the
//! accent roles, and `[tui.colors]` can then override any single role.

use std::collections::BTreeMap;

use ratatui::style::Color;

use super::tokens::{Palette, Token};
use super::Mode;

pub const NAMES: [&str; 4] = ["forge", "ocean", "grove", "rose"];

struct Accents {
    brand: u32,
    hot: u32,
    user: u32,
}

fn accents(name: &str, mode: Mode) -> Option<Accents> {
    let (dark, light) = match name {
        "ocean" => (
            Accents {
                brand: 0x4FC3F7,
                hot: 0x9BE3FF,
                user: 0xFFB454,
            },
            Accents {
                brand: 0x0B6FA8,
                hot: 0x0A7F9E,
                user: 0xB34E09,
            },
        ),
        "grove" => (
            Accents {
                brand: 0x6FCF8E,
                hot: 0xB8F0A8,
                user: 0xE8B84A,
            },
            Accents {
                brand: 0x1F7A45,
                hot: 0x5F7A00,
                user: 0x1A66C7,
            },
        ),
        "rose" => (
            Accents {
                brand: 0xF28FB0,
                hot: 0xFFC2D6,
                user: 0x6FB1FF,
            },
            Accents {
                brand: 0xB8325F,
                hot: 0xC23B7A,
                user: 0x1A66C7,
            },
        ),
        _ => return None,
    };
    Some(match mode {
        Mode::Dark => dark,
        Mode::Light => light,
    })
}

fn rgb(hex: u32) -> Color {
    Color::Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

fn mix(hex: u32, toward: u32, weight: f32) -> u32 {
    let channel = |shift: u32| {
        let a = ((hex >> shift) & 0xFF) as f32;
        let b = ((toward >> shift) & 0xFF) as f32;
        (a + (b - a) * weight).round() as u32
    };
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
}

fn fg(hex: u32) -> Token {
    Token {
        fg: rgb(hex),
        ..plain()
    }
}

fn plain() -> Token {
    Token {
        fg: Color::Reset,
        bg: Color::Reset,
        mods: ratatui::style::Modifier::empty(),
    }
}

pub fn is_known(name: &str) -> bool {
    NAMES.contains(&name)
}

pub fn named(name: &str, mode: Mode, mut base: Palette) -> Option<Palette> {
    if name == "forge" {
        return Some(base);
    }
    let a = accents(name, mode)?;
    let (dark_end, light_end) = match mode {
        Mode::Dark => (0x000000, 0xFFFFFF),
        Mode::Light => (0x000000, 0x000000),
    };
    base.brand = fg(a.brand);
    base.brand_hot = fg(a.hot);
    base.user = fg(a.user);
    base.cat[0] = fg(a.brand);
    base.cat[1] = fg(a.user);
    base.heat = [
        fg(mix(a.brand, dark_end, 0.5)),
        fg(mix(a.brand, dark_end, 0.25)),
        fg(a.brand),
        fg(a.hot),
        fg(mix(a.hot, light_end, 0.5)),
    ];
    Some(base)
}

pub fn parse_hex(value: &str) -> Option<u32> {
    let digits = value.trim().strip_prefix('#')?;
    if digits.len() != 6 {
        return None;
    }
    u32::from_str_radix(digits, 16).ok()
}

/// Applies `[tui.colors]` and returns the entries it could not use (unknown role or bad hex).
pub fn with_overrides(
    mut palette: Palette,
    colors: &BTreeMap<String, String>,
) -> (Palette, Vec<String>) {
    let mut rejected = Vec::new();
    for (role, value) in colors {
        let Some(hex) = parse_hex(value) else {
            rejected.push(format!("{role} = {value:?} is not #RRGGBB"));
            continue;
        };
        let slot = match role.as_str() {
            "fg" => &mut palette.fg,
            "fg_muted" => &mut palette.fg_muted,
            "fg_faint" => &mut palette.fg_faint,
            "line" => &mut palette.line,
            "brand" => &mut palette.brand,
            "brand_hot" => &mut palette.brand_hot,
            "user" => &mut palette.user,
            "ok" => &mut palette.ok,
            "warn" => &mut palette.warn,
            "err" => &mut palette.err,
            "surface" => &mut palette.surface,
            "surface_raised" => &mut palette.surface_raised,
            _ => {
                rejected.push(format!("{role} is not a colour role"));
                continue;
            }
        };
        if matches!(role.as_str(), "surface" | "surface_raised") {
            slot.bg = rgb(hex);
        } else {
            slot.fg = rgb(hex);
        }
    }
    (palette, rejected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::tokens::Palette;
    use crate::theme::ColorTier;

    fn base(mode: Mode) -> Palette {
        Palette::resolve(ColorTier::TrueColor, mode)
    }

    #[test]
    fn forge_keeps_the_base_and_named_palettes_change_only_accents() {
        assert_eq!(
            named("forge", Mode::Dark, base(Mode::Dark)),
            Some(base(Mode::Dark))
        );
        let ocean = named("ocean", Mode::Dark, base(Mode::Dark)).unwrap();
        assert_ne!(ocean.brand, base(Mode::Dark).brand);
        assert_eq!(ocean.fg, base(Mode::Dark).fg);
        assert_eq!(ocean.surface, base(Mode::Dark).surface);
        assert_eq!(ocean.brand.fg, rgb(0x4FC3F7));
        assert!(named("nope", Mode::Dark, base(Mode::Dark)).is_none());
    }

    #[test]
    fn every_listed_palette_resolves_in_both_modes() {
        for name in NAMES {
            for mode in [Mode::Dark, Mode::Light] {
                assert!(named(name, mode, base(mode)).is_some(), "{name}");
            }
        }
    }

    #[test]
    fn overrides_apply_and_bad_entries_are_reported() {
        let mut colors = BTreeMap::new();
        colors.insert("brand".to_string(), "#112233".to_string());
        colors.insert("surface".to_string(), "#000000".to_string());
        colors.insert("err".to_string(), "red".to_string());
        colors.insert("sparkle".to_string(), "#FFFFFF".to_string());
        let (palette, rejected) = with_overrides(base(Mode::Dark), &colors);
        assert_eq!(palette.brand.fg, rgb(0x112233));
        assert_eq!(palette.surface.bg, rgb(0x000000));
        assert_eq!(palette.err, base(Mode::Dark).err);
        assert_eq!(rejected.len(), 2);
    }
}
