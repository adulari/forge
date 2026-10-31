//! `[tui]`: how the interactive chat renders and responds.

use serde::{Deserialize, Serialize};

fn default_true() -> bool {
    true
}

/// Interactive chat TUI rendering mode.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TuiConfig {
    /// Render the chat on the alternate screen (full-screen), with a scrollable transcript and
    /// the panels pinned at the bottom. Default: true. When false, the chat runs inline in the
    /// terminal's native scrollback (a small pinned live region). The `--inline` / `--fullscreen`
    /// flags on `forge chat` override this per-invocation.
    #[serde(default = "default_tui_fullscreen")]
    pub fullscreen: bool,
    /// In full-screen mode, enable minimal mouse reporting (button + wheel, no motion tracking) so
    /// the wheel scrolls the transcript. Default: true. Because motion tracking stays off, the
    /// terminal's native click-drag text selection keeps working. Set false to disable mouse
    /// reporting entirely (e.g. a terminal where any reporting blocks selection). No effect inline.
    #[serde(default = "default_tui_mouse_capture")]
    pub mouse_capture: bool,
    /// Colour mode: `"auto"` (default; detects from `COLORFGBG`), `"dark"` or `"light"`. The
    /// `FORGE_THEME` environment variable overrides it.
    #[serde(default = "default_tui_auto")]
    pub theme: String,
    /// Colour depth: `"auto"` (default; detects from `NO_COLOR`, `COLORTERM` and `TERM`),
    /// `"truecolor"`, `"256"`, `"16"` or `"none"`. The `FORGE_COLOR` environment variable
    /// overrides it.
    #[serde(default = "default_tui_auto")]
    pub color: String,
    /// Glyph set: `"auto"` (default; ASCII on the Linux console, `TERM=dumb` and non-UTF-8 or CJK
    /// locales), `"unicode"` or `"ascii"`. The `FORGE_GLYPHS` environment variable overrides it.
    #[serde(default = "default_tui_auto")]
    pub glyphs: String,
    /// Turn off animation (spinner colour cycling, rail cooling, flashes). Default: false. The
    /// `FORGE_REDUCE_MOTION` environment variable and monochrome terminals also turn it off.
    #[serde(default)]
    pub reduce_motion: bool,
    /// Fold each run of tool calls into one summary row in the full-screen transcript (click or
    /// Enter opens it). Default: true. False shows every call as its own card, as before.
    #[serde(default = "default_true")]
    pub fold_tool_groups: bool,
    /// After a `!command` typed in the composer runs, have the model respond to its output (Claude
    /// Code's `respondToBashCommands`). Default: true. False only adds the run to the context.
    #[serde(default = "default_true")]
    pub respond_to_bash: bool,
    /// Named accent palette for truecolor terminals: `"forge"` (default), `"ocean"`, `"grove"` or
    /// `"rose"`. `/theme <name>` switches it live and saves it here.
    #[serde(default = "default_tui_palette")]
    pub palette: String,
    /// Per-role colour overrides on top of the palette, as `role = "#RRGGBB"`, e.g.
    /// `brand = "#FF8A30"`. Roles: fg, fg_muted, fg_faint, line, brand, brand_hot, user, ok, warn,
    /// err, surface, surface_raised. Truecolor terminals only.
    #[serde(default)]
    pub colors: std::collections::BTreeMap<String, String>,
    /// Transcript lines one mouse-wheel notch scrolls (1-20). Default: 3.
    #[serde(default = "default_tui_scroll_speed")]
    pub scroll_speed: u8,
    /// Text cursor shape: `"auto"` (default; leave the terminal's own), `"block"`, `"bar"` or
    /// `"underline"`. Applies to the terminal cursor, not the in-prompt block.
    #[serde(default = "default_tui_auto")]
    pub cursor: String,
    /// Show the tokens-per-second rate in the status line. Default: true.
    #[serde(default = "default_true")]
    pub show_tps: bool,
}

impl Default for TuiConfig {
    fn default() -> Self {
        Self {
            fullscreen: default_tui_fullscreen(),
            mouse_capture: default_tui_mouse_capture(),
            theme: default_tui_auto(),
            color: default_tui_auto(),
            glyphs: default_tui_auto(),
            reduce_motion: false,
            fold_tool_groups: true,
            respond_to_bash: true,
            palette: default_tui_palette(),
            colors: std::collections::BTreeMap::new(),
            scroll_speed: default_tui_scroll_speed(),
            cursor: default_tui_auto(),
            show_tps: true,
        }
    }
}

fn default_tui_scroll_speed() -> u8 {
    3
}

fn default_tui_palette() -> String {
    "forge".to_string()
}

fn default_tui_auto() -> String {
    "auto".to_string()
}

fn default_tui_fullscreen() -> bool {
    true
}

fn default_tui_mouse_capture() -> bool {
    true
}
