//! Forgeline theme: colour tokens, capability detection, glyph sets and role styles.
//!
//! The theme is resolved once, before the first draw, by [`init`] (from the environment and the
//! `[tui]` config section) and read everywhere through [`t`]. Renderers never write colour
//! literals: they ask for a role (`t().palette.brand`, `styles::muted()`, ...). Tests scope a tier
//! with [`scoped`], and the pure [`detect::resolve`] takes an explicit environment map so
//! detection is testable without touching the process environment.

pub mod backend;
pub mod degrade;
pub mod detect;
pub mod glyphs;
pub mod palettes;
pub mod styles;
pub mod tokens;

/// Role colours as plain `Color` values, for the few places that cannot take a `Style`. Prefer
/// [`styles`], which also carries the modifiers that matter in the 16-colour and mono tiers.
pub mod color {
    use ratatui::style::Color;

    use super::t;

    macro_rules! roles {
        ($($name:ident),* $(,)?) => {
            $(pub fn $name() -> Color {
                let token = t().palette.$name;
                if token.fg != Color::Reset { token.fg } else { token.bg }
            })*
        };
    }
    roles!(
        fg,
        fg_muted,
        fg_faint,
        line,
        brand,
        brand_hot,
        user,
        ok,
        warn,
        err,
        surface,
        surface_raised
    );

    /// Text drawn on a solid brand/role fill (the cursor block, pills). Dark text on dark-mode
    /// fills, white on light-mode fills.
    pub fn on_fill() -> Color {
        use super::ColorTier::*;
        match (t().tier, t().mode) {
            (TrueColor, super::Mode::Dark) => Color::Rgb(16, 16, 20),
            (TrueColor, super::Mode::Light) => Color::Rgb(255, 255, 255),
            (Ansi256, super::Mode::Dark) => Color::Indexed(233),
            (Ansi256, super::Mode::Light) => Color::Indexed(231),
            _ => Color::Reset,
        }
    }

    /// The text-selection highlight background. Mono has no fill, so callers there add REVERSED
    /// through [`super::styles::selection`] instead.
    pub fn selection() -> Color {
        use super::ColorTier::*;
        match (t().tier, t().mode) {
            (TrueColor, super::Mode::Dark) => Color::Rgb(40, 70, 132),
            (TrueColor, super::Mode::Light) => Color::Rgb(176, 208, 255),
            (Ansi256, super::Mode::Dark) => Color::Indexed(24),
            (Ansi256, super::Mode::Light) => Color::Indexed(153),
            (Ansi16, _) => Color::Blue,
            (Mono, _) => Color::Reset,
        }
    }
}

#[cfg(test)]
mod tests;

use std::cell::Cell;
use std::sync::OnceLock;

use forge_config::TuiConfig;

pub use glyphs::Glyphs;
pub use tokens::{Palette, Token};

/// How many colours the terminal can show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorTier {
    TrueColor,
    Ansi256,
    Ansi16,
    /// No colour at all (`NO_COLOR`, `TERM=dumb`): hierarchy is carried by bold, dim and italic.
    Mono,
}

/// Whether the terminal background is dark or light.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Dark,
    Light,
}

/// Which glyph set to draw with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlyphTier {
    Unicode,
    Ascii,
}

/// Whether animation runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motion {
    Full,
    Reduced,
}

/// The resolved, immutable theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub tier: ColorTier,
    pub mode: Mode,
    pub glyph_tier: GlyphTier,
    pub motion: Motion,
    pub palette: Palette,
}

impl Theme {
    pub fn new(tier: ColorTier, mode: Mode, glyph_tier: GlyphTier, motion: Motion) -> Self {
        Self {
            tier,
            mode,
            glyph_tier,
            motion,
            palette: Palette::resolve(tier, mode),
        }
    }

    /// Whether backgrounds may be painted at all (truecolor and 256 only).
    pub fn paints_backgrounds(&self) -> bool {
        matches!(self.tier, ColorTier::TrueColor | ColorTier::Ansi256)
    }

    pub fn glyphs(&self) -> &'static Glyphs {
        glyphs::set(self.glyph_tier)
    }

    pub fn animates(&self) -> bool {
        self.motion == Motion::Full
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::new(
            ColorTier::TrueColor,
            Mode::Dark,
            GlyphTier::Unicode,
            Motion::Full,
        )
    }
}

static GLOBAL: OnceLock<Theme> = OnceLock::new();

thread_local! {
    static OVERRIDE: Cell<Option<Theme>> = const { Cell::new(None) };
}

/// Resolve the theme from the process environment and `[tui]` config. Call once, before the
/// first draw or the first scrollback insert. Later calls are ignored.
pub fn init(config: &TuiConfig) -> Theme {
    let theme = *GLOBAL.get_or_init(|| detect::resolve(&detect::process_env(), config));
    let _ = set_palette(&config.palette, &config.colors);
    theme
}

static RUNTIME_PALETTE: std::sync::RwLock<Option<Palette>> = std::sync::RwLock::new(None);

/// Switch the accent palette for the rest of the process. Returns the `[tui.colors]` entries that
/// were ignored. Only truecolor terminals take a named palette or overrides.
pub fn set_palette(
    name: &str,
    colors: &std::collections::BTreeMap<String, String>,
) -> Result<Vec<String>, String> {
    let base =
        *GLOBAL.get_or_init(|| detect::resolve(&detect::process_env(), &TuiConfig::default()));
    let name = name.trim().to_lowercase();
    if !palettes::is_known(&name) {
        return Err(format!(
            "unknown palette {name:?}; choose one of {}",
            palettes::NAMES.join(", ")
        ));
    }
    if base.tier != ColorTier::TrueColor {
        return if name == "forge" && colors.is_empty() {
            Ok(Vec::new())
        } else {
            Err("named palettes need a truecolor terminal".to_string())
        };
    }
    let named = palettes::named(&name, base.mode, base.palette)
        .ok_or_else(|| format!("unknown palette {name:?}"))?;
    let (palette, rejected) = palettes::with_overrides(named, colors);
    if let Ok(mut slot) = RUNTIME_PALETTE.write() {
        *slot = (palette != base.palette).then_some(palette);
    }
    Ok(rejected)
}

/// The active theme: a [`scoped`] override on this thread, else the initialised global. Without
/// `init` it falls back to detection from the environment with default config (and, under
/// `cfg(test)`, to the deterministic truecolor/dark/unicode default).
pub fn t() -> Theme {
    if let Some(theme) = OVERRIDE.with(Cell::get) {
        return theme;
    }
    if let Some(theme) = GLOBAL.get() {
        let mut theme = *theme;
        if let Some(palette) = RUNTIME_PALETTE.read().ok().and_then(|slot| *slot) {
            theme.palette = palette;
        }
        return theme;
    }
    #[cfg(test)]
    {
        Theme::default()
    }
    #[cfg(not(test))]
    {
        *GLOBAL.get_or_init(|| detect::resolve(&detect::process_env(), &TuiConfig::default()))
    }
}

/// Run `f` with `theme` active on this thread (tests, and rendering to a fixed tier).
pub fn scoped<R>(theme: Theme, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<Theme>);
    impl Drop for Restore {
        fn drop(&mut self) {
            OVERRIDE.with(|o| o.set(self.0));
        }
    }
    let _restore = Restore(OVERRIDE.with(|o| o.replace(Some(theme))));
    f()
}

/// A third-party RGB colour (syntect output) as a ratatui colour. The remapping backend brings it
/// down to the active tier at draw time.
pub fn rgb(r: u8, g: u8, b: u8) -> ratatui::style::Color {
    ratatui::style::Color::Rgb(r, g, b)
}

/// SGR escape sequence for `token` in the active tier, for output written outside ratatui
/// (startup banner and errors). Mono emits only bold/dim.
pub fn ansi(token: Token) -> String {
    let theme = t();
    let mut codes: Vec<String> = Vec::new();
    use ratatui::style::Modifier;
    if token.mods.contains(Modifier::BOLD) {
        codes.push("1".into());
    }
    if token.mods.contains(Modifier::DIM) {
        codes.push("2".into());
    }
    if token.mods.contains(Modifier::ITALIC) {
        codes.push("3".into());
    }
    if theme.tier != ColorTier::Mono {
        if let Some(code) = degrade::sgr_fg(token.fg) {
            codes.push(code);
        }
    }
    if codes.is_empty() {
        String::new()
    } else {
        format!("\x1b[{}m", codes.join(";"))
    }
}

/// `text` wrapped in the SGR for `token` and a reset, or plain `text` when the token is plain.
pub fn paint(token: Token, text: &str) -> String {
    let open = ansi(token);
    if open.is_empty() {
        text.to_string()
    } else {
        format!("{open}{text}\x1b[0m")
    }
}

/// A startup error in the notice grammar: `✗ message`, hanging-indented detail lines, then a
/// `Next:` line. `colored` is false when the stream is not a terminal; the tier (and so
/// `NO_COLOR`) decides everything else. Used before the TUI starts, in both display modes.
pub fn render_error(message: &str, details: &[String], next: &str, colored: bool) -> String {
    let theme = t();
    let glyph = theme.glyphs().fail;
    let paint_if = |token: Token, text: &str| {
        if colored {
            paint(token, text)
        } else {
            text.to_string()
        }
    };
    let err = Token {
        mods: theme.palette.err.mods | ratatui::style::Modifier::BOLD,
        ..theme.palette.err
    };
    let mut out = format!("{} {message}", paint_if(err, glyph));
    for detail in details {
        out.push_str(&format!("\n  {}", paint_if(theme.palette.fg_muted, detail)));
    }
    out.push_str(&format!(
        "\n  {} {next}",
        paint_if(theme.palette.fg_muted, "Next:")
    ));
    out
}
