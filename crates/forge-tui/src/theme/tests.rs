//! Theme tests: detection matrix, per-tier token values, the colour safety net, contrast, glyphs,
//! reduced motion, and the source scan that keeps colour literals inside the theme.

use std::collections::HashMap;

use forge_config::TuiConfig;
use ratatui::backend::{Backend, TestBackend};
use ratatui::buffer::Cell;
use ratatui::style::{Color, Modifier};

use super::backend::RemapBackend;
use super::detect::{self, resolve};
use super::{degrade, glyphs, styles, ColorTier, GlyphTier, Mode, Motion, Palette, Theme};

fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

fn cfg() -> TuiConfig {
    TuiConfig::default()
}

fn tier(pairs: &[(&str, &str)]) -> ColorTier {
    detect::color_tier(&env(pairs), &cfg())
}

#[test]
fn colour_tier_follows_the_documented_precedence() {
    use ColorTier::*;
    // FORGE_COLOR names the tier outright, and beats NO_COLOR.
    assert_eq!(
        tier(&[("FORGE_COLOR", "truecolor"), ("NO_COLOR", "1")]),
        TrueColor
    );
    assert_eq!(tier(&[("FORGE_COLOR", "256")]), Ansi256);
    assert_eq!(tier(&[("FORGE_COLOR", "16")]), Ansi16);
    assert_eq!(
        tier(&[("FORGE_COLOR", "none"), ("COLORTERM", "truecolor")]),
        Mono
    );
    // NO_COLOR set and non-empty means mono; empty does not count.
    assert_eq!(tier(&[("NO_COLOR", "1"), ("COLORTERM", "truecolor")]), Mono);
    assert_eq!(
        tier(&[("NO_COLOR", ""), ("COLORTERM", "truecolor")]),
        TrueColor
    );
    // TERM=dumb is mono even with COLORTERM.
    assert_eq!(tier(&[("TERM", "dumb"), ("COLORTERM", "truecolor")]), Mono);
    // COLORTERM wins over a 16-colour TERM.
    assert_eq!(
        tier(&[("TERM", "xterm"), ("COLORTERM", "24bit")]),
        TrueColor
    );
    // Known truecolor terminal programs.
    for program in ["iTerm.app", "WezTerm", "vscode", "ghostty"] {
        assert_eq!(
            tier(&[("TERM", "xterm"), ("TERM_PROGRAM", program)]),
            TrueColor,
            "{program}"
        );
    }
    assert_eq!(
        tier(&[("TERM", "xterm"), ("TERM_PROGRAM", "Apple_Terminal")]),
        Ansi256
    );
    // TERM families.
    for term in ["xterm-256color", "tmux-256color", "screen-256color"] {
        assert_eq!(tier(&[("TERM", term)]), Ansi256, "{term}");
    }
    for term in [
        "linux", "xterm", "screen", "vt100", "vt220", "ansi", "cygwin",
    ] {
        assert_eq!(tier(&[("TERM", term)]), Ansi16, "{term}");
    }
    assert_eq!(tier(&[("TERM", "alacritty")]), Ansi256);
    assert_eq!(tier(&[]), Ansi256);
}

#[test]
fn config_colour_sits_between_the_env_override_and_detection() {
    let config = TuiConfig {
        color: "16".into(),
        ..cfg()
    };
    let colored = |pairs: &[(&str, &str)]| detect::color_tier(&env(pairs), &config);
    assert_eq!(colored(&[("COLORTERM", "truecolor")]), ColorTier::Ansi16);
    assert_eq!(colored(&[("FORGE_COLOR", "256")]), ColorTier::Ansi256);
    // A config value is explicit, so it also beats NO_COLOR.
    assert_eq!(colored(&[("NO_COLOR", "1")]), ColorTier::Ansi16);
}

fn mode(pairs: &[(&str, &str)]) -> Mode {
    detect::mode(&env(pairs), &cfg())
}

#[test]
fn light_and_dark_follow_forge_theme_then_colorfgbg() {
    assert_eq!(mode(&[]), Mode::Dark);
    assert_eq!(mode(&[("FORGE_THEME", "light")]), Mode::Light);
    assert_eq!(
        mode(&[("FORGE_THEME", "dark"), ("COLORFGBG", "0;15")]),
        Mode::Dark
    );
    for light in ["0;15", "0;7", "default;15", "0;default;15", "0;12"] {
        let want = if light.ends_with("default") {
            Mode::Dark
        } else {
            Mode::Light
        };
        assert_eq!(mode(&[("COLORFGBG", light)]), want, "{light}");
    }
    for dark in ["15;0", "15;1", "15;6", "7;8", "15;default;0"] {
        assert_eq!(mode(&[("COLORFGBG", dark)]), Mode::Dark, "{dark}");
    }
    let config = TuiConfig {
        theme: "light".into(),
        ..cfg()
    };
    assert_eq!(
        detect::mode(&env(&[("COLORFGBG", "15;0")]), &config),
        Mode::Light
    );
    assert_eq!(
        detect::mode(&env(&[("FORGE_THEME", "dark")]), &config),
        Mode::Dark,
        "the env override beats config"
    );
}

fn glyph_tier(pairs: &[(&str, &str)]) -> GlyphTier {
    detect::glyph_tier(&env(pairs), &cfg())
}

#[test]
fn glyph_tier_falls_back_to_ascii_where_unicode_would_misrender() {
    use GlyphTier::*;
    assert_eq!(glyph_tier(&[]), Unicode);
    assert_eq!(glyph_tier(&[("LANG", "en_US.UTF-8")]), Unicode);
    assert_eq!(glyph_tier(&[("LANG", "en_US.utf8")]), Unicode);
    assert_eq!(
        glyph_tier(&[("FORGE_GLYPHS", "ascii"), ("LANG", "en_US.UTF-8")]),
        Ascii
    );
    assert_eq!(
        glyph_tier(&[("FORGE_GLYPHS", "unicode"), ("TERM", "linux")]),
        Unicode
    );
    for term in ["linux", "vt100", "vt220", "dumb"] {
        assert_eq!(glyph_tier(&[("TERM", term)]), Ascii, "{term}");
    }
    // The first locale variable that is set decides, in LC_ALL, LC_CTYPE, LANG order.
    assert_eq!(glyph_tier(&[("LANG", "C")]), Ascii);
    assert_eq!(
        glyph_tier(&[("LC_ALL", "C"), ("LANG", "en_US.UTF-8")]),
        Ascii
    );
    assert_eq!(
        glyph_tier(&[("LC_ALL", "en_US.UTF-8"), ("LANG", "C")]),
        Unicode
    );
    // CJK locales draw ambiguous-width glyphs two cells wide.
    for locale in ["ja_JP.UTF-8", "ko_KR.UTF-8", "zh_CN.UTF-8"] {
        assert_eq!(glyph_tier(&[("LANG", locale)]), Ascii, "{locale}");
    }
}

#[test]
fn motion_is_reduced_by_env_config_or_mono() {
    let motion = |pairs: &[(&str, &str)], config: &TuiConfig, tier| {
        detect::motion(&env(pairs), config, tier)
    };
    let c = cfg();
    assert_eq!(motion(&[], &c, ColorTier::TrueColor), Motion::Full);
    assert_eq!(
        motion(&[("FORGE_REDUCE_MOTION", "1")], &c, ColorTier::TrueColor),
        Motion::Reduced
    );
    assert_eq!(
        motion(&[("FORGE_REDUCE_MOTION", "0")], &c, ColorTier::TrueColor),
        Motion::Full
    );
    let reduced = TuiConfig {
        reduce_motion: true,
        ..cfg()
    };
    assert_eq!(motion(&[], &reduced, ColorTier::Ansi256), Motion::Reduced);
    assert_eq!(motion(&[], &c, ColorTier::Mono), Motion::Reduced);
}

#[test]
fn resolve_assembles_all_four_axes() {
    let theme = resolve(
        &env(&[
            ("TERM", "xterm-256color"),
            ("COLORFGBG", "0;15"),
            ("LANG", "en_US.UTF-8"),
        ]),
        &cfg(),
    );
    assert_eq!(theme.tier, ColorTier::Ansi256);
    assert_eq!(theme.mode, Mode::Light);
    assert_eq!(theme.glyph_tier, GlyphTier::Unicode);
    assert_eq!(theme.motion, Motion::Full);
    assert_eq!(
        theme.palette,
        Palette::resolve(ColorTier::Ansi256, Mode::Light)
    );
}

fn fg_row(p: &Palette) -> [Color; 10] {
    [
        p.fg.fg,
        p.fg_muted.fg,
        p.fg_faint.fg,
        p.line.fg,
        p.brand.fg,
        p.brand_hot.fg,
        p.user.fg,
        p.ok.fg,
        p.warn.fg,
        p.err.fg,
    ]
}

#[test]
fn every_token_resolves_to_its_table_value_in_every_tier() {
    use Color::{Blue, DarkGray, Green, Indexed as I, LightYellow, Red, Reset, Rgb, Yellow};
    let rgb = |h: u32| Rgb((h >> 16) as u8, (h >> 8) as u8, h as u8);
    let cases: Vec<(ColorTier, Mode, [Color; 10])> = vec![
        (
            ColorTier::TrueColor,
            Mode::Dark,
            [
                rgb(0xD8DCE8),
                rgb(0x9096AA),
                rgb(0x626779),
                rgb(0x3A3C4A),
                rgb(0xFF8A30),
                rgb(0xFFC247),
                rgb(0x6FB1FF),
                rgb(0x5CD07A),
                rgb(0xE8B84A),
                rgb(0xF26464),
            ],
        ),
        (
            ColorTier::TrueColor,
            Mode::Light,
            [
                rgb(0x1D2030),
                rgb(0x565B6B),
                rgb(0x8B8D98),
                rgb(0xD3D0C8),
                rgb(0xB34E09),
                rgb(0xB86A00),
                rgb(0x1A66C7),
                rgb(0x17793A),
                rgb(0x8F6200),
                rgb(0xC7312F),
            ],
        ),
        (
            ColorTier::Ansi256,
            Mode::Dark,
            [
                I(253),
                I(246),
                I(242),
                I(238),
                I(208),
                I(214),
                I(75),
                I(78),
                I(179),
                I(203),
            ],
        ),
        (
            ColorTier::Ansi256,
            Mode::Light,
            [
                I(235),
                I(240),
                I(245),
                I(252),
                I(130),
                I(136),
                I(26),
                I(28),
                I(94),
                I(160),
            ],
        ),
        (
            ColorTier::Ansi16,
            Mode::Dark,
            [
                Reset,
                Reset,
                DarkGray,
                DarkGray,
                Yellow,
                LightYellow,
                Blue,
                Green,
                LightYellow,
                Red,
            ],
        ),
        (
            ColorTier::Ansi16,
            Mode::Light,
            [
                Reset, Reset, DarkGray, DarkGray, Yellow, Yellow, Blue, Green, Yellow, Red,
            ],
        ),
        (ColorTier::Mono, Mode::Dark, [Reset; 10]),
    ];
    for (tier, mode, want) in cases {
        let palette = Palette::resolve(tier, mode);
        assert_eq!(fg_row(&palette), want, "{tier:?} {mode:?}");
    }
}

#[test]
fn modifiers_carry_meaning_where_colour_cannot() {
    let dark16 = Palette::resolve(ColorTier::Ansi16, Mode::Dark);
    assert!(dark16.fg_muted.mods.contains(Modifier::DIM));
    assert!(dark16.warn.mods.contains(Modifier::BOLD));
    let light16 = Palette::resolve(ColorTier::Ansi16, Mode::Light);
    assert!(light16.brand.mods.contains(Modifier::BOLD));
    let mono = Palette::resolve(ColorTier::Mono, Mode::Dark);
    assert!(mono.fg_muted.mods.contains(Modifier::DIM));
    assert!(mono.fg_faint.mods.contains(Modifier::DIM));
    assert!(mono.brand.mods.contains(Modifier::BOLD));
    assert!(mono.err.mods.contains(Modifier::BOLD));
    assert!(mono.warn.mods.contains(Modifier::BOLD));
}

#[test]
fn only_truecolor_and_256_paint_backgrounds() {
    for (tier, paints) in [
        (ColorTier::TrueColor, true),
        (ColorTier::Ansi256, true),
        (ColorTier::Ansi16, false),
        (ColorTier::Mono, false),
    ] {
        let palette = Palette::resolve(tier, Mode::Dark);
        let fills = [
            palette.surface,
            palette.surface_raised,
            palette.diff_add_bg,
            palette.diff_del_bg,
        ];
        for fill in fills {
            assert_eq!(fill.bg != Color::Reset, paints, "{tier:?}");
        }
    }
}

#[test]
fn the_safety_net_maps_rgb_to_the_nearest_palette_colour() {
    // Ember orange lands on xterm's orange, and pure primaries on their ANSI twins.
    assert_eq!(degrade::nearest_256([255, 138, 48]), 208);
    assert_eq!(degrade::nearest_256([0, 0, 0]), 16);
    assert_eq!(degrade::nearest_256([255, 255, 255]), 231);
    assert_eq!(degrade::nearest_16([255, 0, 0]), Color::LightRed);
    assert_eq!(degrade::nearest_16([0, 0, 0]), Color::Black);
    assert_eq!(degrade::nearest_16([250, 250, 250]), Color::White);
    assert_eq!(
        degrade::map_color(Color::Rgb(255, 255, 255), ColorTier::Ansi256),
        Color::Indexed(231)
    );
    assert_eq!(
        degrade::map_color(Color::Indexed(196), ColorTier::Ansi16),
        Color::LightRed
    );
    assert_eq!(
        degrade::map_color(Color::Rgb(1, 2, 3), ColorTier::TrueColor),
        Color::Rgb(1, 2, 3)
    );
    assert_eq!(
        degrade::map_color(Color::Rgb(1, 2, 3), ColorTier::Mono),
        Color::Reset
    );
    // Named colours are already tier-safe in the 16-colour tier.
    assert_eq!(
        degrade::map_color(Color::Green, ColorTier::Ansi16),
        Color::Green
    );
}

fn drawn_cell(tier: ColorTier, fg: Color, modifier: Modifier) -> Cell {
    let mut backend = RemapBackend::with_tier(TestBackend::new(4, 1), tier);
    let mut cell = Cell::new("x");
    cell.fg = fg;
    cell.bg = Color::Rgb(10, 20, 30);
    cell.modifier = modifier;
    backend
        .draw([(0u16, 0u16, &cell)].into_iter())
        .expect("draw");
    backend.inner().buffer()[(0, 0)].clone()
}

#[test]
fn the_remapping_backend_applies_the_tier_and_keeps_modifiers() {
    let cell = drawn_cell(
        ColorTier::Ansi256,
        Color::Rgb(255, 255, 255),
        Modifier::BOLD,
    );
    assert_eq!(cell.fg, Color::Indexed(231));
    assert!(matches!(cell.bg, Color::Indexed(_)));
    assert!(cell.modifier.contains(Modifier::BOLD));

    let cell = drawn_cell(ColorTier::Ansi16, Color::Rgb(255, 0, 0), Modifier::DIM);
    assert_eq!(cell.fg, Color::LightRed);
    assert!(cell.modifier.contains(Modifier::DIM));

    let cell = drawn_cell(
        ColorTier::Mono,
        Color::Rgb(255, 0, 0),
        Modifier::BOLD | Modifier::ITALIC,
    );
    assert_eq!((cell.fg, cell.bg), (Color::Reset, Color::Reset));
    assert!(cell.modifier.contains(Modifier::BOLD | Modifier::ITALIC));

    let cell = drawn_cell(ColorTier::TrueColor, Color::Rgb(9, 9, 9), Modifier::empty());
    assert_eq!(cell.fg, Color::Rgb(9, 9, 9));
}

fn luminance(c: Color) -> f64 {
    let Color::Rgb(r, g, b) = c else {
        panic!("contrast is checked on truecolor tokens, got {c:?}")
    };
    let lin = |v: u8| {
        let v = f64::from(v) / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
}

fn contrast(fg: Color, bg: Color) -> f64 {
    let (a, b) = (luminance(fg), luminance(bg));
    let (hi, lo) = if a > b { (a, b) } else { (b, a) };
    (hi + 0.05) / (lo + 0.05)
}

#[test]
fn text_tokens_meet_their_contrast_floor_on_both_backgrounds() {
    for (mode, bg) in [
        (Mode::Dark, Color::Rgb(0x10, 0x10, 0x14)),
        (Mode::Light, Color::Rgb(0xFF, 0xFF, 0xFF)),
    ] {
        let p = Palette::resolve(ColorTier::TrueColor, mode);
        for (name, token) in [
            ("fg", p.fg),
            ("fg_muted", p.fg_muted),
            ("brand", p.brand),
            ("user", p.user),
            ("ok", p.ok),
            ("warn", p.warn),
            ("err", p.err),
        ] {
            let ratio = contrast(token.fg, bg);
            assert!(ratio >= 4.5, "{mode:?} {name} is {ratio:.2}:1");
        }
        let ratio = contrast(p.fg_faint.fg, bg);
        assert!(ratio >= 3.0, "{mode:?} fg_faint is {ratio:.2}:1");
    }
}

#[test]
fn reduced_motion_freezes_the_spinner_cooling_and_cycling() {
    let reduced = Theme::new(
        ColorTier::TrueColor,
        Mode::Dark,
        GlyphTier::Unicode,
        Motion::Reduced,
    );
    super::scoped(reduced, || {
        assert_eq!(
            glyphs::g().spin(0),
            glyphs::g().spin(7),
            "static spinner glyph"
        );
        assert_eq!(styles::spinner(0), styles::spinner(9));
        assert_eq!(
            styles::effort(forge_types::EffortLevel::WhiteHot, 0),
            styles::effort(forge_types::EffortLevel::WhiteHot, 11)
        );
        // A rail that would be cooling reads as fully settled.
        assert_eq!(
            styles::rail(styles::RailKind::Settled(0)),
            styles::rail(styles::RailKind::Settled(2))
        );
    });
    super::scoped(Theme::default(), || {
        assert_ne!(glyphs::g().spin(0), glyphs::g().spin(1));
        assert_ne!(styles::spinner(0), styles::spinner(8));
        assert_ne!(
            styles::rail(styles::RailKind::Settled(0)),
            styles::rail(styles::RailKind::Settled(2))
        );
    });
}

#[test]
fn scoped_themes_nest_and_restore() {
    let ascii = Theme::new(
        ColorTier::Mono,
        Mode::Dark,
        GlyphTier::Ascii,
        Motion::Reduced,
    );
    super::scoped(ascii, || {
        assert_eq!(super::t().tier, ColorTier::Mono);
        super::scoped(Theme::default(), || {
            assert_eq!(super::t().tier, ColorTier::TrueColor);
        });
        assert_eq!(super::t().tier, ColorTier::Mono);
    });
    assert_eq!(super::t().tier, ColorTier::TrueColor);
}

#[test]
fn ascii_glyphs_are_all_ascii_and_unicode_ones_are_single_width() {
    use unicode_width::UnicodeWidthStr;
    for (set, ascii) in [
        (glyphs::set(GlyphTier::Ascii), true),
        (glyphs::set(GlyphTier::Unicode), false),
    ] {
        let singles = [
            set.brand,
            set.rail_user,
            set.rail_assistant,
            set.rail_decision,
            set.prompt,
            set.disclose_closed,
            set.disclose_open,
            set.card_gutter,
            set.awaiting,
            set.ok,
            set.fail,
            set.warn,
            set.info,
            set.group,
            set.live_dot,
            set.task_open,
            set.task_active,
            set.task_done,
            set.gauge_full,
            set.gauge_empty,
            set.sep,
            set.diff_minus,
            set.caret,
        ];
        for glyph in singles {
            if ascii {
                assert!(glyph.is_ascii(), "{glyph}");
            } else {
                assert_eq!(glyph.width(), 1, "{glyph}");
            }
        }
        for frame in set.spinner {
            assert_eq!(frame.is_ascii(), ascii);
        }
    }
}

#[test]
fn error_notices_use_the_tier_glyph_and_honour_colour() {
    super::scoped(Theme::default(), || {
        let plain = super::render_error("boom", &["cause one".into()], "forge doctor", false);
        assert_eq!(plain, "✗ boom\n  cause one\n  Next: forge doctor");
        assert!(super::render_error("boom", &[], "x", true).contains("\x1b["));
    });
    let mono = Theme::new(
        ColorTier::Mono,
        Mode::Dark,
        GlyphTier::Ascii,
        Motion::Reduced,
    );
    super::scoped(mono, || {
        let out = super::render_error("boom", &[], "forge doctor", true);
        assert!(out.starts_with("\x1b[1mx\x1b[0m boom"), "{out:?}");
        assert!(!out.contains("38;"), "no colour escapes in mono: {out:?}");
    });
}

/// Colour literals live in the theme, so a palette change is a one-file change and every tier
/// stays consistent. `ansi_line.rs` converts SGR from external statusline commands and is data,
/// not a palette.
#[test]
fn no_colour_literals_outside_the_theme() {
    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("read src dir").flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    walk(&src, &mut files);
    let mut offenders = Vec::new();
    for file in files {
        let rel = file
            .strip_prefix(&src)
            .expect("under src")
            .to_string_lossy()
            .replace('\\', "/");
        if rel.starts_with("theme/") || rel == "ansi_line.rs" {
            continue;
        }
        let text = std::fs::read_to_string(&file).expect("read source");
        for (n, line) in text.lines().enumerate() {
            let mut rest = line;
            while let Some(i) = rest.find("Color::") {
                let after = &rest[i + "Color::".len()..];
                let name: String = after.chars().take_while(|c| c.is_alphanumeric()).collect();
                if !name.is_empty() && name != "Reset" {
                    offenders.push(format!("{rel}:{}: Color::{name}", n + 1));
                }
                rest = &rest[i + "Color::".len()..];
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "colour literals outside theme/: {offenders:#?}"
    );
}

#[test]
fn normalise_regyphs_retired_glyphs_and_folds_to_ascii_where_needed() {
    super::scoped(Theme::default(), || {
        assert_eq!(
            glyphs::normalise("⚠ careful ✖ broke ⚒ forge"),
            "▲ careful ✗ broke ✦ forge"
        );
        assert_eq!(
            glyphs::normalise("✓ fine — ok ↑3"),
            "✓ fine — ok ↑3",
            "other glyphs stay"
        );
    });
    let ascii = Theme::new(
        ColorTier::Mono,
        Mode::Dark,
        GlyphTier::Ascii,
        Motion::Reduced,
    );
    super::scoped(ascii, || {
        let folded = glyphs::normalise("⚠ careful ✖ broke ✓ ok ↑72 ↓30 · wait… │ ╭─╮ 日本");
        assert_eq!(
            folded,
            "! careful x broke + ok up 72 down 30 - wait... | +-+ ??"
        );
        assert!(folded.is_ascii());
    });
}
