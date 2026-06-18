//! The `forge init` first-run setup wizard: a full-screen, animated ratatui flow to enable
//! providers (enter API keys, masked) and pick the subscription plan backing each installed CLI
//! bridge. Pure [`State`] + transitions are unit-tested; [`run`] is the thin terminal I/O shell.
//! Keys are returned to the caller to store in the OS keyring — never written to disk here.

use std::collections::HashMap;
use std::io;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use forge_types::{CreditMode, PermissionMode};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use ratatui::{Frame, Terminal};

const ORANGE: Color = Color::Rgb(255, 145, 60);
const USER: Color = Color::Rgb(125, 180, 255);
const DIM: Color = Color::Rgb(110, 110, 120);
const OKGREEN: Color = Color::Rgb(120, 210, 140);
const WARNYEL: Color = Color::Rgb(235, 200, 110);
const TOOLCYAN: Color = Color::Rgb(120, 200, 215);

/// A key-based provider offered in the wizard.
pub struct ProviderItem {
    pub id: String,
    pub label: String,
    pub had_key: bool,
}

/// An installed CLI bridge offered a plan choice: `plans` is `(human label, stored slug)`.
pub struct BridgeItem {
    pub prefix: String,
    pub plans: Vec<(String, String)>,
}

/// Everything the composition root (forge-cli) feeds the wizard.
#[derive(Default)]
pub struct WizardInput {
    pub providers: Vec<ProviderItem>,
    /// Search-API providers (e.g. `brave`) for the `web_search` tool — their own section.
    pub search: Vec<ProviderItem>,
    pub bridges: Vec<BridgeItem>,
    /// Current default permission mode (pre-fills the picker).
    pub current_permission: PermissionMode,
    /// Current credit conservation mode (pre-fills the picker).
    pub current_credit_mode: CreditMode,
}

/// What the wizard collected on finish.
#[derive(Debug, Default, PartialEq)]
pub struct WizardOutcome {
    /// `(provider, key)` to store in the OS keyring.
    pub keys: Vec<(String, String)>,
    /// Bridge prefix → chosen plan slug.
    pub plans: HashMap<String, String>,
    /// Updated default permission mode.
    pub permission: PermissionMode,
    /// Updated credit conservation mode.
    pub credit_mode: CreditMode,
    pub cancelled: bool,
}

/// A focusable row: a provider key field, a search-key field, a bridge plan chooser, a settings
/// row, or the Finish button.
#[derive(Clone, Copy, PartialEq)]
enum Row {
    Provider(usize),
    Search(usize),
    Bridge(usize),
    Permission,
    CreditMode,
    Finish,
}

/// Pure wizard state (no I/O), so navigation/editing is unit-testable.
struct State {
    input: WizardInput,
    /// Entered key per provider (empty = leave as-is / skip).
    keys: Vec<String>,
    /// Entered key per search provider (empty = leave as-is / skip).
    search_keys: Vec<String>,
    /// Selected plan index per bridge.
    plan_sel: Vec<Option<usize>>,
    /// Index into PermissionMode::all() for the settings row.
    perm_idx: usize,
    /// Index into CreditMode::all() for the settings row.
    credit_idx: usize,
    cursor: usize,
    /// True while typing into the focused key field (provider or search).
    editing: bool,
    /// Reveal animation 0.0→1.0.
    anim: f32,
    done: bool,
    cancelled: bool,
}

impl State {
    fn new(input: WizardInput) -> Self {
        let keys = vec![String::new(); input.providers.len()];
        let search_keys = vec![String::new(); input.search.len()];
        let plan_sel = vec![None; input.bridges.len()];
        let perm_idx = PermissionMode::all()
            .iter()
            .position(|&m| m == input.current_permission)
            .unwrap_or(0);
        let credit_idx = CreditMode::all()
            .iter()
            .position(|&m| m == input.current_credit_mode)
            .unwrap_or(0);
        Self {
            input,
            keys,
            search_keys,
            plan_sel,
            perm_idx,
            credit_idx,
            cursor: 0,
            editing: false,
            anim: 0.0,
            done: false,
            cancelled: false,
        }
    }

    /// The full focusable row list: providers, search keys, installed bridges, settings, Finish.
    fn rows(&self) -> Vec<Row> {
        let mut r: Vec<Row> = (0..self.input.providers.len()).map(Row::Provider).collect();
        r.extend((0..self.input.search.len()).map(Row::Search));
        r.extend((0..self.input.bridges.len()).map(Row::Bridge));
        r.push(Row::Permission);
        r.push(Row::CreditMode);
        r.push(Row::Finish);
        r
    }

    /// The focused editable key field (a provider or search-provider key), if any.
    fn key_field_mut(&mut self) -> Option<&mut String> {
        match self.focused() {
            Row::Provider(i) => self.keys.get_mut(i),
            Row::Search(i) => self.search_keys.get_mut(i),
            _ => None,
        }
    }

    fn focused(&self) -> Row {
        self.rows()[self.cursor.min(self.rows().len() - 1)]
    }

    fn move_up(&mut self) {
        if !self.editing {
            self.cursor = self.cursor.saturating_sub(1);
        }
    }

    fn move_down(&mut self) {
        if !self.editing {
            self.cursor = (self.cursor + 1).min(self.rows().len() - 1);
        }
    }

    /// Enter: start/stop editing a provider key, cycle settings, advance bridge plan, or finish.
    fn enter(&mut self) {
        match self.focused() {
            Row::Provider(_) | Row::Search(_) => self.editing = !self.editing,
            Row::Bridge(i) => {
                let n = self.input.bridges[i].plans.len();
                if n > 0 {
                    let next = self.plan_sel[i].map(|s| (s + 1) % n).unwrap_or(0);
                    self.plan_sel[i] = Some(next);
                }
            }
            Row::Permission => {
                self.perm_idx = (self.perm_idx + 1) % PermissionMode::all().len();
            }
            Row::CreditMode => {
                self.credit_idx = (self.credit_idx + 1) % CreditMode::all().len();
            }
            Row::Finish => self.done = true,
        }
    }

    /// A digit selects the Nth plan when a bridge row is focused, or cycles settings rows.
    fn digit(&mut self, d: u32) {
        match self.focused() {
            Row::Bridge(i) => {
                let idx = (d as usize).wrapping_sub(1);
                if idx < self.input.bridges[i].plans.len() {
                    self.plan_sel[i] = Some(idx);
                }
            }
            Row::Permission => {
                let idx = (d as usize).wrapping_sub(1);
                if idx < PermissionMode::all().len() {
                    self.perm_idx = idx;
                }
            }
            Row::CreditMode => {
                let idx = (d as usize).wrapping_sub(1);
                if idx < CreditMode::all().len() {
                    self.credit_idx = idx;
                }
            }
            _ => {}
        }
    }

    fn push_char(&mut self, c: char) {
        if self.editing {
            if let Some(field) = self.key_field_mut() {
                field.push(c);
            }
        } else if c.is_ascii_digit() {
            self.digit(c.to_digit(10).unwrap());
        }
    }

    fn backspace(&mut self) {
        if self.editing {
            if let Some(field) = self.key_field_mut() {
                field.pop();
            }
        }
    }

    /// Esc: stop editing if mid-key, else cancel the whole wizard.
    fn escape(&mut self) {
        if self.editing {
            self.editing = false;
        } else {
            self.cancelled = true;
            self.done = true;
        }
    }

    fn tick(&mut self) {
        if self.anim < 1.0 {
            self.anim = (self.anim + 0.12).min(1.0);
        }
    }

    fn outcome(&self) -> WizardOutcome {
        if self.cancelled {
            return WizardOutcome {
                cancelled: true,
                permission: self.input.current_permission,
                credit_mode: self.input.current_credit_mode,
                ..Default::default()
            };
        }
        let keys = self
            .input
            .providers
            .iter()
            .zip(&self.keys)
            .chain(self.input.search.iter().zip(&self.search_keys))
            .filter(|(_, k)| !k.is_empty())
            .map(|(p, k)| (p.id.clone(), k.clone()))
            .collect();
        let plans = self
            .input
            .bridges
            .iter()
            .zip(&self.plan_sel)
            .filter_map(|(b, sel)| sel.map(|i| (b.prefix.clone(), b.plans[i].1.clone())))
            .collect();
        WizardOutcome {
            keys,
            plans,
            permission: PermissionMode::all()[self.perm_idx],
            credit_mode: CreditMode::all()[self.credit_idx],
            cancelled: false,
        }
    }
}

/// Run the wizard against `input`, returning what the user chose. Enters the alternate screen +
/// raw mode and restores them on exit (and on panic, via the shared restore hook).
pub fn run(input: WizardInput) -> io::Result<WizardOutcome> {
    crate::driver::install_panic_restore();
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    crossterm::execute!(stdout, EnterAlternateScreen)?;
    let result = run_loop(input);
    let _ = disable_raw_mode();
    let _ = crossterm::execute!(io::stdout(), LeaveAlternateScreen);
    result
}

fn run_loop(input: WizardInput) -> io::Result<WizardOutcome> {
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    let mut state = State::new(input);
    loop {
        terminal.draw(|f| render(f, &state))?;
        if state.done {
            return Ok(state.outcome());
        }
        // Poll with a short timeout so the reveal animation advances even without input.
        if event::poll(Duration::from_millis(60))? {
            if let Event::Key(k) = event::read()? {
                if k.kind != KeyEventKind::Release {
                    match k.code {
                        KeyCode::Up => state.move_up(),
                        KeyCode::Down => state.move_down(),
                        KeyCode::Enter => state.enter(),
                        KeyCode::Esc => state.escape(),
                        KeyCode::Backspace => state.backspace(),
                        KeyCode::Char(c) => state.push_char(c),
                        _ => {}
                    }
                }
            }
        } else {
            state.tick();
        }
    }
}

fn render(f: &mut Frame, state: &State) {
    let area = f.area();
    f.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ORANGE))
        .title(Span::styled(
            " ⚒ Forge setup ",
            Style::default().fg(ORANGE).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        "Enable providers (Enter to type a key, masked) · pick your bridge plan · then Finish.",
        Style::default().fg(DIM),
    )));
    lines.push(Line::from(Span::styled(
        "↑/↓ move · Enter edit/cycle · digits pick a plan · Esc cancel",
        Style::default().fg(DIM),
    )));
    lines.push(Line::from(""));

    let rows = state.rows();
    // Reveal rows progressively with the open animation.
    let revealed = ((state.anim * rows.len() as f32).ceil() as usize).clamp(1, rows.len());

    lines.push(section("Providers", USER));
    for (vi, row) in rows.iter().enumerate().take(revealed) {
        if let Row::Provider(i) = row {
            let p = &state.input.providers[*i];
            lines.push(key_line(
                p,
                &state.keys[*i],
                vi == state.cursor,
                state.editing,
            ));
        }
    }
    if !state.input.search.is_empty() {
        lines.push(Line::from(""));
        lines.push(section("Search (web_search key)", WARNYEL));
        for (vi, row) in rows.iter().enumerate().take(revealed) {
            if let Row::Search(i) = row {
                let p = &state.input.search[*i];
                lines.push(key_line(
                    p,
                    &state.search_keys[*i],
                    vi == state.cursor,
                    state.editing,
                ));
            }
        }
    }
    if !state.input.bridges.is_empty() {
        lines.push(Line::from(""));
        lines.push(section("Subscription bridges", TOOLCYAN));
        for (vi, row) in rows.iter().enumerate().take(revealed) {
            if let Row::Bridge(i) = row {
                lines.push(bridge_line(state, *i, vi == state.cursor));
            }
        }
    }
    lines.push(Line::from(""));
    lines.push(section("Settings", WARNYEL));
    for (vi, row) in rows.iter().enumerate().take(revealed) {
        match row {
            Row::Permission => lines.push(cycle_line(
                "permission",
                PermissionMode::all()
                    .iter()
                    .map(|m| (m.label(), m.description()))
                    .collect::<Vec<_>>(),
                state.perm_idx,
                vi == state.cursor,
            )),
            Row::CreditMode => lines.push(cycle_line(
                "credit mode",
                CreditMode::all()
                    .iter()
                    .map(|m| (m.label(), m.description()))
                    .collect::<Vec<_>>(),
                state.credit_idx,
                vi == state.cursor,
            )),
            _ => {}
        }
    }
    lines.push(Line::from(""));
    let finish_selected = matches!(state.focused(), Row::Finish);
    let finish_style = if finish_selected {
        Style::default().fg(OKGREEN).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(OKGREEN)
    };
    lines.push(Line::from(Span::styled(
        format!("{} Finish setup", marker(finish_selected)),
        finish_style,
    )));

    f.render_widget(Paragraph::new(lines).alignment(Alignment::Left), pad(inner));
}

fn section(title: &str, color: Color) -> Line<'static> {
    Line::from(Span::styled(
        format!("  {title}"),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    ))
}

fn marker(selected: bool) -> &'static str {
    if selected {
        "▸"
    } else {
        " "
    }
}

/// Render one key field (a provider or a search provider) — they look identical.
fn key_line(p: &ProviderItem, key: &str, selected: bool, state_editing: bool) -> Line<'static> {
    let editing = selected && state_editing;
    let value = if editing {
        format!("{}▌", "•".repeat(key.len()))
    } else if !key.is_empty() {
        format!("{} set", "•".repeat(key.len().min(8)))
    } else if p.had_key {
        "[already configured]".to_string()
    } else {
        "(Enter to add a key)".to_string()
    };
    let name_color = if selected { ORANGE } else { USER };
    let val_color = if editing {
        WARNYEL
    } else if !key.is_empty() || p.had_key {
        OKGREEN
    } else {
        DIM
    };
    Line::from(vec![
        Span::styled(
            format!("  {} {:<14}", marker(selected), p.id),
            Style::default().fg(name_color),
        ),
        Span::styled(format!("{:<42} ", p.label), Style::default().fg(DIM)),
        Span::styled(value, Style::default().fg(val_color)),
    ])
}

fn bridge_line(state: &State, i: usize, selected: bool) -> Line<'static> {
    let b = &state.input.bridges[i];
    let chosen = state.plan_sel[i]
        .map(|s| b.plans[s].0.as_str())
        .unwrap_or("(choose)");
    let opts = b
        .plans
        .iter()
        .enumerate()
        .map(|(n, (label, _))| format!("{})_{}", n + 1, label).replace('_', " "))
        .collect::<Vec<_>>()
        .join("  ");
    let name_color = if selected { ORANGE } else { TOOLCYAN };
    Line::from(vec![
        Span::styled(
            format!("  {} {:<12}", marker(selected), b.prefix),
            Style::default().fg(name_color),
        ),
        Span::styled(
            format!("{:<14}", chosen),
            Style::default().fg(OKGREEN).add_modifier(Modifier::BOLD),
        ),
        Span::styled(opts, Style::default().fg(DIM)),
    ])
}

/// Render a cycling picker row (Enter/digit cycles through options).
fn cycle_line(
    name: &'static str,
    options: Vec<(&'static str, &'static str)>,
    sel: usize,
    selected: bool,
) -> Line<'static> {
    let chosen = options[sel.min(options.len() - 1)].0;
    let desc = options[sel.min(options.len() - 1)].1;
    let opts = options
        .iter()
        .enumerate()
        .map(|(n, (label, _))| format!("{}){}", n + 1, label))
        .collect::<Vec<_>>()
        .join("  ");
    let name_color = if selected { ORANGE } else { WARNYEL };
    Line::from(vec![
        Span::styled(
            format!("  {} {:<14}", marker(selected), name),
            Style::default().fg(name_color),
        ),
        Span::styled(
            format!("{:<14}", chosen),
            Style::default().fg(OKGREEN).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("{:<38} ", desc), Style::default().fg(DIM)),
        Span::styled(opts, Style::default().fg(DIM)),
    ])
}

/// Inset the content one column/row inside the border.
fn pad(area: Rect) -> Rect {
    Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> WizardInput {
        WizardInput {
            providers: vec![
                ProviderItem {
                    id: "anthropic".into(),
                    label: "Claude API".into(),
                    had_key: false,
                },
                ProviderItem {
                    id: "groq".into(),
                    label: "Groq free".into(),
                    had_key: true,
                },
            ],
            search: vec![ProviderItem {
                id: "brave".into(),
                label: "Brave Search".into(),
                had_key: false,
            }],
            bridges: vec![BridgeItem {
                prefix: "claude-cli".into(),
                plans: vec![
                    ("Pro".into(), "pro".into()),
                    ("Max 20×".into(), "max-20x".into()),
                ],
            }],
            current_permission: PermissionMode::default(),
            current_credit_mode: CreditMode::default(),
        }
    }

    #[test]
    fn rows_are_providers_then_search_then_bridges_then_settings_then_finish() {
        let s = State::new(input());
        let rows = s.rows();
        assert!(matches!(rows[0], Row::Provider(0)));
        assert!(matches!(rows[2], Row::Search(0)));
        assert!(matches!(rows[3], Row::Bridge(0)));
        assert!(matches!(rows[4], Row::Permission));
        assert!(matches!(rows[5], Row::CreditMode));
        assert!(matches!(rows[6], Row::Finish));
    }

    #[test]
    fn typing_a_search_key_collects_it() {
        let mut s = State::new(input());
        s.move_down(); // Provider(1)
        s.move_down(); // Search(0)
        assert!(matches!(s.focused(), Row::Search(0)));
        s.enter(); // start editing
        for c in "brave-key".chars() {
            s.push_char(c);
        }
        s.enter(); // commit
        let out = s.outcome();
        assert!(out
            .keys
            .contains(&("brave".to_string(), "brave-key".to_string())));
    }

    #[test]
    fn typing_a_key_collects_it_in_the_outcome() {
        let mut s = State::new(input());
        // Focus provider 0, start editing, type a key, commit.
        s.enter();
        assert!(s.editing);
        for c in "sk-test".chars() {
            s.push_char(c);
        }
        s.enter(); // commit
        assert!(!s.editing);
        let out = s.outcome();
        assert_eq!(
            out.keys,
            vec![("anthropic".to_string(), "sk-test".to_string())]
        );
    }

    #[test]
    fn digit_selects_a_bridge_plan() {
        let mut s = State::new(input());
        // Move to the bridge row (providers ×2, search ×1, then the bridge at index 3).
        s.move_down();
        s.move_down();
        s.move_down();
        assert!(matches!(s.focused(), Row::Bridge(0)));
        s.push_char('2'); // pick "Max 20×"
        let out = s.outcome();
        assert_eq!(
            out.plans.get("claude-cli").map(String::as_str),
            Some("max-20x")
        );
    }

    #[test]
    fn esc_cancels_and_yields_no_keys() {
        let mut s = State::new(input());
        s.escape();
        assert!(s.done);
        let out = s.outcome();
        assert!(out.cancelled && out.keys.is_empty() && out.plans.is_empty());
    }

    #[test]
    fn navigation_clamps_at_the_ends() {
        let mut s = State::new(input());
        s.move_up(); // already at top
        assert_eq!(s.cursor, 0);
        for _ in 0..20 {
            s.move_down();
        }
        assert!(matches!(s.focused(), Row::Finish));
    }
}
