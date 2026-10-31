//! Interface preferences and small modes that do not belong to one panel: wheel speed, cursor
//! shape, the tokens-per-second toggle, the Quiet / Default / Everything verbosity cycle and the
//! rotating prompt examples.

use super::App;

/// How the in-prompt cursor is drawn (`[tui] cursor`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CursorShape {
    #[default]
    Block,
    Bar,
    Underline,
}

impl CursorShape {
    /// `"auto"` and anything unknown keep the block.
    pub fn parse(text: &str) -> Self {
        match text.trim().to_ascii_lowercase().as_str() {
            "bar" => Self::Bar,
            "underline" => Self::Underline,
            _ => Self::Block,
        }
    }
}

/// How much the transcript says. Cycled with the `verbosity_cycle` key (Alt+V).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Verbosity {
    /// Reasoning hidden, tool runs folded, informational notices left out.
    Quiet,
    #[default]
    Default,
    /// Reasoning shown and every tool run open.
    Everything,
}

impl Verbosity {
    pub fn next(self) -> Self {
        match self {
            Self::Quiet => Self::Default,
            Self::Default => Self::Everything,
            Self::Everything => Self::Quiet,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Quiet => "quiet",
            Self::Default => "default",
            Self::Everything => "everything",
        }
    }

    fn blurb(self) -> &'static str {
        match self {
            Self::Quiet => "reasoning hidden, tool runs folded, notices trimmed",
            Self::Default => "reasoning hidden, tool runs folded",
            Self::Everything => "reasoning shown, every tool run open",
        }
    }
}

#[derive(Debug, Clone)]
pub struct PolishState {
    /// Transcript lines per mouse-wheel notch (`[tui] scroll_speed`).
    pub scroll_step: usize,
    pub cursor: CursorShape,
    /// Show the tokens-per-second rate in the status line (`[tui] show_tps`).
    pub show_tps: bool,
    pub verbosity: Verbosity,
    /// Where the rotation of prompt examples starts, so two sessions do not open on the same one.
    seed: usize,
    pub qr: Option<crate::qr_overlay::QrOverlay>,
}

impl Default for PolishState {
    fn default() -> Self {
        Self {
            scroll_step: 3,
            cursor: CursorShape::Block,
            show_tps: true,
            verbosity: Verbosity::Default,
            seed: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() as usize),
            qr: None,
        }
    }
}

/// Prompts the composer suggests while it is empty, in the order they rotate.
const PROMPT_EXAMPLES: [&str; 8] = [
    "fix the failing test in this repo",
    "explain how this project works",
    "review my uncommitted changes",
    "write tests for what I changed",
    "find where errors are logged",
    "summarize the last five commits",
    "simplify the longest function",
    "add a --verbose flag to the CLI",
];

/// What `!` runs, offered once a bare `!` is typed.
const SHELL_EXAMPLES: [&str; 6] = [
    "git status",
    "cargo test",
    "ls -la",
    "git log --oneline -10",
    "npm run build",
    "docker ps",
];

impl App {
    /// Apply `[tui]` preferences at startup.
    pub fn apply_tui_prefs(&mut self, tui: &forge_config::TuiConfig) {
        self.polish.scroll_step = usize::from(tui.scroll_speed).clamp(1, 20);
        self.polish.cursor = CursorShape::parse(&tui.cursor);
        self.polish.show_tps = tui.show_tps;
    }

    /// Lines one wheel notch moves.
    pub fn scroll_step(&self) -> usize {
        self.polish.scroll_step
    }

    /// The suggestion shown in an empty composer, rotating as the conversation grows.
    pub(crate) fn prompt_example(&self) -> &'static str {
        PROMPT_EXAMPLES[(self.polish.seed + self.main_log.len()) % PROMPT_EXAMPLES.len()]
    }

    /// The command suggested after a bare `!`.
    pub(crate) fn shell_example(&self) -> &'static str {
        SHELL_EXAMPLES[(self.polish.seed + self.main_log.len()) % SHELL_EXAMPLES.len()]
    }

    /// Step Quiet, Default, Everything and apply it to what the transcript shows.
    pub fn cycle_verbosity(&mut self) {
        let next = self.polish.verbosity.next();
        self.set_verbosity(next);
    }

    pub fn set_verbosity(&mut self, level: Verbosity) {
        // The announcement is sent before a Quiet level starts trimming notices, so it is seen.
        self.polish.verbosity = Verbosity::Default;
        self.note(&format!(
            "verbosity: {}  ({})",
            level.label(),
            level.blurb()
        ));
        self.polish.verbosity = level;
        self.show_thinking = level == Verbosity::Everything;
        if self.fold_enabled() {
            self.set_tool_groups_open(level == Verbosity::Everything);
        }
    }

    fn set_tool_groups_open(&mut self, open: bool) {
        for _ in 0..2 {
            let spans = self.group_spans();
            let all_open = spans.iter().all(|s| self.group_expanded(s.id));
            let all_shut = spans.iter().all(|s| !self.group_expanded(s.id));
            if (open && all_open) || (!open && all_shut) {
                return;
            }
            self.toggle_all_tool_groups();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verbosity_cycles_through_all_three_levels() {
        let mut level = Verbosity::Quiet;
        let mut seen = Vec::new();
        for _ in 0..4 {
            level = level.next();
            seen.push(level.label());
        }
        assert_eq!(seen, ["default", "everything", "quiet", "default"]);
    }

    #[test]
    fn cursor_shapes_parse() {
        assert_eq!(CursorShape::parse("Bar"), CursorShape::Bar);
        assert_eq!(CursorShape::parse("underline"), CursorShape::Underline);
        assert_eq!(CursorShape::parse("auto"), CursorShape::Block);
        assert_eq!(CursorShape::parse("nonsense"), CursorShape::Block);
    }

    #[test]
    fn examples_rotate_with_the_conversation() {
        let mut app = App::default();
        let first = app.prompt_example();
        let mut seen = std::collections::HashSet::new();
        for i in 0..PROMPT_EXAMPLES.len() {
            app.main_log.clear();
            for _ in 0..i {
                app.main_log.push(Default::default());
            }
            seen.insert(app.prompt_example());
        }
        assert_eq!(seen.len(), PROMPT_EXAMPLES.len());
        assert!(PROMPT_EXAMPLES.contains(&first));
        assert!(SHELL_EXAMPLES.contains(&app.shell_example()));
    }

    #[test]
    fn everything_shows_reasoning_and_quiet_hides_it() {
        let mut app = App::default();
        app.set_verbosity(Verbosity::Everything);
        assert!(app.show_thinking);
        assert_eq!(app.polish.verbosity, Verbosity::Everything);
        app.set_verbosity(Verbosity::Quiet);
        assert!(!app.show_thinking);
        assert_eq!(app.polish.verbosity, Verbosity::Quiet);
    }
}
