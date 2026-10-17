//! Capability detection: colour tier, light/dark mode, glyph set and motion, from an explicit
//! environment map plus the `[tui]` config. [`resolve`] is pure so the whole rule table is
//! testable without mutating the process environment.

use std::collections::HashMap;

use forge_config::TuiConfig;

use super::{ColorTier, GlyphTier, Mode, Motion, Theme};

/// The environment variables detection reads, copied out of the process environment.
pub fn process_env() -> HashMap<String, String> {
    const KEYS: &[&str] = &[
        "NO_COLOR",
        "FORGE_COLOR",
        "FORGE_THEME",
        "FORGE_GLYPHS",
        "FORGE_REDUCE_MOTION",
        "COLORTERM",
        "COLORFGBG",
        "TERM",
        "TERM_PROGRAM",
        "LC_ALL",
        "LC_CTYPE",
        "LANG",
    ];
    KEYS.iter()
        .filter_map(|k| std::env::var(k).ok().map(|v| ((*k).to_string(), v)))
        .collect()
}

fn get<'a>(env: &'a HashMap<String, String>, key: &str) -> Option<&'a str> {
    env.get(key).map(String::as_str).filter(|v| !v.is_empty())
}

fn lower(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn explicit_tier(value: &str) -> Option<ColorTier> {
    match lower(value).as_str() {
        "truecolor" | "24bit" | "true" => Some(ColorTier::TrueColor),
        "256" => Some(ColorTier::Ansi256),
        "16" => Some(ColorTier::Ansi16),
        "none" | "mono" | "off" => Some(ColorTier::Mono),
        _ => None,
    }
}

pub fn color_tier(env: &HashMap<String, String>, config: &TuiConfig) -> ColorTier {
    if let Some(tier) = get(env, "FORGE_COLOR").and_then(explicit_tier) {
        return tier;
    }
    if let Some(tier) = explicit_tier(&config.color) {
        return tier;
    }
    if get(env, "NO_COLOR").is_some() {
        return ColorTier::Mono;
    }
    let term = get(env, "TERM").map(lower).unwrap_or_default();
    if term == "dumb" {
        return ColorTier::Mono;
    }
    if matches!(
        get(env, "COLORTERM").map(lower).as_deref(),
        Some("truecolor" | "24bit")
    ) {
        return ColorTier::TrueColor;
    }
    match get(env, "TERM_PROGRAM") {
        Some("iTerm.app" | "WezTerm" | "vscode" | "ghostty") => return ColorTier::TrueColor,
        Some("Apple_Terminal") => return ColorTier::Ansi256,
        _ => {}
    }
    if term.contains("256color") {
        return ColorTier::Ansi256;
    }
    if matches!(
        term.as_str(),
        "linux" | "xterm" | "screen" | "vt100" | "vt220" | "ansi" | "cygwin"
    ) {
        return ColorTier::Ansi16;
    }
    ColorTier::Ansi256
}

fn colorfgbg_mode(value: &str) -> Option<Mode> {
    let last = value.rsplit(';').next()?.trim();
    let n: u32 = last.parse().ok()?;
    match n {
        0..=6 | 8 => Some(Mode::Dark),
        _ => Some(Mode::Light),
    }
}

pub fn mode(env: &HashMap<String, String>, config: &TuiConfig) -> Mode {
    let explicit = |value: &str| match lower(value).as_str() {
        "dark" => Some(Mode::Dark),
        "light" => Some(Mode::Light),
        _ => None,
    };
    if let Some(mode) = get(env, "FORGE_THEME").and_then(explicit) {
        return mode;
    }
    if let Some(mode) = explicit(&config.theme) {
        return mode;
    }
    get(env, "COLORFGBG")
        .and_then(colorfgbg_mode)
        .unwrap_or(Mode::Dark)
}

fn is_cjk_locale(locale: &str) -> bool {
    let lang = locale.split(['_', '.', '@']).next().unwrap_or("");
    matches!(lang, "ja" | "ko" | "zh")
}

pub fn glyph_tier(env: &HashMap<String, String>, config: &TuiConfig) -> GlyphTier {
    let explicit = |value: &str| match lower(value).as_str() {
        "unicode" => Some(GlyphTier::Unicode),
        "ascii" => Some(GlyphTier::Ascii),
        _ => None,
    };
    if let Some(tier) = get(env, "FORGE_GLYPHS").and_then(explicit) {
        return tier;
    }
    if let Some(tier) = explicit(&config.glyphs) {
        return tier;
    }
    let term = get(env, "TERM").map(lower).unwrap_or_default();
    if matches!(term.as_str(), "linux" | "vt100" | "vt220" | "dumb") {
        return GlyphTier::Ascii;
    }
    let locale = ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .find_map(|k| get(env, k));
    if let Some(locale) = locale {
        let l = lower(locale);
        if !(l.contains("utf-8") || l.contains("utf8")) || is_cjk_locale(&l) {
            return GlyphTier::Ascii;
        }
    }
    GlyphTier::Unicode
}

pub fn motion(env: &HashMap<String, String>, config: &TuiConfig, tier: ColorTier) -> Motion {
    let env_off = get(env, "FORGE_REDUCE_MOTION")
        .map(|v| !matches!(lower(v).as_str(), "0" | "false" | "no" | "off"))
        .unwrap_or(false);
    if env_off || config.reduce_motion || tier == ColorTier::Mono {
        Motion::Reduced
    } else {
        Motion::Full
    }
}

/// Resolve the whole theme from an explicit environment and config.
pub fn resolve(env: &HashMap<String, String>, config: &TuiConfig) -> Theme {
    let tier = color_tier(env, config);
    Theme::new(
        tier,
        mode(env, config),
        glyph_tier(env, config),
        motion(env, config, tier),
    )
}
