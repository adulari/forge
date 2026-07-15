//! Read-only fullscreen command and keyboard-reference viewer.

use std::io;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use forge_config::KeybindsConfig;
use ratatui::backend::CrosstermBackend;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Terminal;

use crate::commands::COMMANDS;
use crate::keybind_configurator::{action_desc, all_actions};
use crate::keybinds::combo_display;
use crate::surface::{self, ACCENT, DIM, TEXT, TOOLCYAN};

/// The reference page opened initially. Both pages remain one Tab press away from the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelpTab {
    Commands,
    Keybinds,
}

impl HelpTab {
    fn toggle(self) -> Self {
        match self {
            Self::Commands => Self::Keybinds,
            Self::Keybinds => Self::Commands,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Commands => "Commands",
            Self::Keybinds => "Keyboard shortcuts",
        }
    }
}

fn command_group(name: &str) -> &'static str {
    match name {
        "help" | "keys" | "config" | "mode" | "thinking" | "effort" | "statusline" => "Session",
        "sessions" | "replay" | "resume" | "new" | "undo" | "checkpoint" | "checkpoints"
        | "compact" | "uncompact" => "History",
        "model" | "models" | "usage" | "mesh" | "duel" => "Models & routing",
        "plan" | "execute" | "goal" | "pr" | "loop" | "workflow" | "assay" | "lattice" => "Work",
        _ => "Project & tools",
    }
}

fn command_reference_lines() -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(Span::styled(
        "Start with a plain request. Use / only when you need a specific control.",
        Style::default().fg(DIM),
    ))];
    let mut previous_group = None;
    for command in COMMANDS {
        let group = command_group(command.name);
        if previous_group != Some(group) {
            lines.push(Line::default());
            lines.push(Line::from(Span::styled(
                group,
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            )));
            previous_group = Some(group);
        }
        lines.push(Line::from(vec![
            Span::styled(
                format!("  /{:<14}", command.name),
                Style::default().fg(TEXT),
            ),
            Span::styled(command.desc, Style::default().fg(DIM)),
        ]));
        let basic_usage = format!("/{}", command.name);
        if command.usage != basic_usage {
            lines.push(Line::from(Span::styled(
                format!("      {}", command.usage),
                Style::default().fg(TOOLCYAN),
            )));
        }
    }
    lines
}

fn keybind_reference_lines(keybinds: &KeybindsConfig) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(Span::styled(
        "F1 or ? opens this page. Change bindings with /keys, then Ctrl+K if you need an action.",
        Style::default().fg(DIM),
    ))];
    lines.push(Line::default());
    lines.push(Line::from(Span::styled(
        "Keyboard shortcuts",
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
    )));
    lines.extend(all_actions().into_iter().map(|action| {
        let combo = keybinds
            .binds
            .get(action)
            .map(combo_display)
            .unwrap_or_else(|| "(unset)".to_string());
        Line::from(vec![
            Span::styled(format!("  {:<22}", combo), Style::default().fg(TEXT)),
            Span::styled(action_desc(action), Style::default().fg(DIM)),
        ])
    }));
    lines
}

fn reference_lines(keybinds: &KeybindsConfig, tab: HelpTab) -> Vec<Line<'static>> {
    match tab {
        HelpTab::Commands => command_reference_lines(),
        HelpTab::Keybinds => keybind_reference_lines(keybinds),
    }
}

/// Run the read-only command and keyboard reference viewer.
pub fn run_help(keybinds: &KeybindsConfig, initial_tab: HelpTab) -> io::Result<()> {
    enable_raw_mode()?;
    crossterm::execute!(io::stdout(), EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    terminal.clear()?;
    let mut tab = initial_tab;
    let mut scroll = 0u16;
    loop {
        terminal.draw(|frame| {
            let area = frame.area();
            surface::render_backdrop(frame, area);
            let inner = surface::render_panel(
                frame,
                area,
                surface::title(tab.title(), surface::SurfaceTone::Accent),
                Some(surface::hint(
                    "Tab switch · ↑/↓ scroll · PgUp/PgDn page · Esc/q close",
                )),
                surface::SurfaceTone::Accent,
            );
            frame.render_widget(
                Paragraph::new(Text::from(reference_lines(keybinds, tab)))
                    .wrap(Wrap { trim: true })
                    .scroll((scroll, 0)),
                inner,
            );
        })?;
        if let Event::Key(key) = event::read()? {
            if key.kind == KeyEventKind::Release {
                continue;
            }
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => break,
                KeyCode::Up | KeyCode::Char('k') => scroll = scroll.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => scroll = scroll.saturating_add(1),
                KeyCode::PageUp => scroll = scroll.saturating_sub(10),
                KeyCode::PageDown => scroll = scroll.saturating_add(10),
                KeyCode::Tab | KeyCode::BackTab => {
                    tab = tab.toggle();
                    scroll = 0;
                }
                _ => {}
            }
        }
    }
    disable_raw_mode()?;
    crossterm::execute!(io::stdout(), LeaveAlternateScreen)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_reference_includes_registry_commands() {
        let lines = reference_lines(&KeybindsConfig::default(), HelpTab::Commands);
        let text = lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert!(text.contains("/help"));
        assert!(text.contains("/assay [--diff"));
        assert!(text.contains("Models & routing"));
    }

    #[test]
    fn keybind_reference_includes_configured_combos() {
        let lines = reference_lines(&KeybindsConfig::default(), HelpTab::Keybinds);
        let text = lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert!(text.contains("F1"));
    }
}
