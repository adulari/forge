//! Read-only fullscreen command and keyboard-reference viewer.

use std::io;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use forge_config::KeybindsConfig;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};
use ratatui::Terminal;

use crate::app::{ACCENT, DIM, TEXT};
use crate::commands::COMMANDS;
use crate::keybind_configurator::{action_desc, all_actions};
use crate::keybinds::combo_display;

fn reference_lines(keybinds: &KeybindsConfig) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(Span::styled(
        "Slash commands",
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
    ))];
    lines.extend(COMMANDS.iter().map(|command| {
        Line::from(vec![
            Span::styled(
                format!("  {:<22}", command.usage),
                Style::default().fg(TEXT),
            ),
            Span::styled(command.desc, Style::default().fg(DIM)),
        ])
    }));
    lines.push(Line::default());
    lines.push(Line::from(Span::styled(
        "Keybindings",
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

/// Run the read-only command and keyboard reference viewer.
pub fn run_help(keybinds: &KeybindsConfig) -> io::Result<()> {
    enable_raw_mode()?;
    crossterm::execute!(io::stdout(), EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let lines = reference_lines(keybinds);
    let mut scroll = 0u16;
    loop {
        terminal.draw(|frame| {
            let area = frame.area();
            frame.render_widget(Clear, area);
            let outer = Block::default()
                .title(Span::styled(" Help ", Style::default().fg(ACCENT).bold()))
                .border_type(BorderType::Rounded)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(DIM));
            let inner = outer.inner(area);
            frame.render_widget(outer, area);
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(1), Constraint::Length(1)])
                .split(inner);
            frame.render_widget(
                Paragraph::new(Text::from(lines.clone()))
                    .wrap(Wrap { trim: false })
                    .scroll((scroll, 0)),
                chunks[0],
            );
            frame.render_widget(
                Paragraph::new(Span::styled(
                    "↑/↓ scroll · PgUp/PgDn page · Esc/q close",
                    Style::default().fg(DIM),
                )),
                chunks[1],
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
    fn reference_includes_registry_commands_and_configured_combos() {
        let lines = reference_lines(&KeybindsConfig::default());
        let text = lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert!(text.contains("/help"));
        assert!(text.contains("F1"));
    }
}
