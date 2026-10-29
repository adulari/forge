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

/// The provider families the fleet and the statusline tell apart by a one-cell mark.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderFamily {
    Claude,
    OpenAi,
    Gemini,
    OpenCode,
    Grok,
    Kimi,
    Qwen,
    Ollama,
    OpenRouter,
    Bedrock,
    Other,
}

fn family_of_name(name: &str) -> ProviderFamily {
    let name = name.to_ascii_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|n| name.contains(n));
    if has(&["openrouter"]) {
        ProviderFamily::OpenRouter
    } else if has(&["opencode"]) {
        ProviderFamily::OpenCode
    } else if has(&["bedrock"]) {
        ProviderFamily::Bedrock
    } else if has(&["claude", "anthropic"]) {
        ProviderFamily::Claude
    } else if has(&["codex", "openai", "gpt"]) {
        ProviderFamily::OpenAi
    } else if has(&["gemini", "google"]) {
        ProviderFamily::Gemini
    } else if has(&["grok", "xai"]) {
        ProviderFamily::Grok
    } else if has(&["kimi", "moonshot"]) {
        ProviderFamily::Kimi
    } else if has(&["qwen", "dashscope"]) {
        ProviderFamily::Qwen
    } else if has(&["ollama"]) {
        ProviderFamily::Ollama
    } else {
        ProviderFamily::Other
    }
}

/// The family of a routed model id (`namespace::model`) or a bare provider id. The namespace
/// decides; a custom namespace (`inferhub::...`) falls back to what the model name says.
pub fn provider_family(id: &str) -> ProviderFamily {
    match id.split_once("::") {
        Some((namespace, model)) => match family_of_name(namespace) {
            ProviderFamily::Other => family_of_name(model),
            family => family,
        },
        None => family_of_name(id),
    }
}

impl Glyphs {
    /// A one-cell mark for `id`'s provider and the categorical colour slot it is drawn in, so the
    /// same provider reads the same on every surface. ASCII tiers get a letter.
    pub fn provider(&self, id: &str) -> (&'static str, usize) {
        let (unicode, ascii, slot) = match provider_family(id) {
            ProviderFamily::Claude => ("◈", "C", 1),
            ProviderFamily::OpenAi => ("◍", "O", 2),
            ProviderFamily::Gemini => ("✧", "G", 3),
            ProviderFamily::OpenCode => ("▣", "P", 4),
            ProviderFamily::Grok => ("◪", "X", 5),
            ProviderFamily::Kimi => ("◐", "K", 0),
            ProviderFamily::Qwen => ("◓", "Q", 2),
            ProviderFamily::Ollama => ("◒", "L", 3),
            ProviderFamily::OpenRouter => ("⇄", "R", 4),
            ProviderFamily::Bedrock => ("▤", "B", 5),
            ProviderFamily::Other => ("◇", "-", 0),
        };
        (if self.unicode { unicode } else { ascii }, slot)
    }
}

#[cfg(test)]
mod provider_tests {
    use super::*;

    #[test]
    fn the_namespace_decides_and_a_custom_one_falls_back_to_the_model_name() {
        assert_eq!(
            provider_family("claude-cli::sonnet"),
            ProviderFamily::Claude
        );
        assert_eq!(
            provider_family("codex-oauth::gpt-5.5"),
            ProviderFamily::OpenAi
        );
        assert_eq!(
            provider_family("bedrock::global.moonshotai.kimi-k3"),
            ProviderFamily::Bedrock
        );
        assert_eq!(
            provider_family("openrouter::qwen/qwen3"),
            ProviderFamily::OpenRouter
        );
        assert_eq!(provider_family("opencode::muse"), ProviderFamily::OpenCode);
        assert_eq!(
            provider_family("inferhub::cb/gemini-flash"),
            ProviderFamily::Gemini
        );
        assert_eq!(
            provider_family("inferhub::cb/deepseek-v4.1-flash"),
            ProviderFamily::Other
        );
        assert_eq!(provider_family("mock"), ProviderFamily::Other);
    }

    #[test]
    fn every_family_has_a_single_cell_mark_in_both_tiers() {
        for id in [
            "claude",
            "codex",
            "gemini",
            "opencode",
            "grok",
            "kimi",
            "qwen",
            "ollama",
            "openrouter",
            "bedrock",
            "other",
        ] {
            for tier in [GlyphTier::Unicode, GlyphTier::Ascii] {
                let (mark, slot) = set(tier).provider(id);
                assert_eq!(mark.chars().count(), 1, "{id} {tier:?}");
                assert!(slot < 6);
                if tier == GlyphTier::Ascii {
                    assert!(mark.is_ascii());
                }
            }
        }
    }
}
