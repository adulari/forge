//! Pure, testable TUI state and rendering. `App` folds [`PresenterEvent`]s into state;
//! `render` draws that state with ratatui. Both are free of terminal I/O so they can be
//! exercised offline with ratatui's `TestBackend`.

use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line as TextLine, Span};
use ratatui::widgets::{Block, List, ListItem, Paragraph};
use ratatui::Frame;

use crate::PresenterEvent;

const FORGE_ORANGE: Color = Color::Rgb(255, 140, 60);

/// The Mesh routing decision currently displayed.
#[derive(Debug, Clone, Default)]
pub struct RoutingView {
    pub tier: String,
    pub model: String,
    pub rationale: String,
}

/// One rendered line in the conversation transcript.
#[derive(Debug, Clone)]
pub enum Line {
    Assistant(String),
    ToolStart {
        name: String,
        args: String,
    },
    ToolResult {
        name: String,
        ok: bool,
        summary: String,
    },
}

/// All state the TUI needs to render a session.
#[derive(Debug, Clone, Default)]
pub struct App {
    pub session_id: String,
    pub routing: Option<RoutingView>,
    pub lines: Vec<Line>,
    pub cost_usd: f64,
    pub warnings: Vec<String>,
    pub done: bool,
    /// A pending permission question shown while the TUI blocks on the user's y/n.
    pub prompt: Option<String>,
}

impl App {
    /// Fold one presenter event into the view state.
    pub fn apply(&mut self, event: PresenterEvent) {
        match event {
            PresenterEvent::SessionStarted { id } => self.session_id = id,
            PresenterEvent::Routing {
                tier,
                model,
                rationale,
            } => {
                self.routing = Some(RoutingView {
                    tier,
                    model,
                    rationale,
                })
            }
            PresenterEvent::AssistantText(text) => self.lines.push(Line::Assistant(text)),
            PresenterEvent::Warning(msg) => self.warnings.push(msg),
            PresenterEvent::ToolStart { name, args } => {
                self.lines.push(Line::ToolStart { name, args })
            }
            PresenterEvent::ToolResult { name, ok, summary } => {
                self.lines.push(Line::ToolResult { name, ok, summary })
            }
            PresenterEvent::Cost { session_total_usd } => self.cost_usd = session_total_usd,
            PresenterEvent::Done { .. } => self.done = true,
        }
    }
}

/// Draw the whole UI for the current state.
pub fn render(frame: &mut Frame, app: &App) {
    // status box: 2 border rows + routing + cost + per-warning + optional prompt + done hint.
    let status_h = 4u16
        .saturating_add(app.warnings.len().min(u16::MAX as usize) as u16)
        .saturating_add(app.prompt.is_some() as u16)
        .saturating_add(app.done as u16);
    let areas = Layout::vertical([
        Constraint::Length(1),        // header
        Constraint::Min(1),           // conversation
        Constraint::Length(status_h), // mesh status
    ])
    .split(frame.area());

    render_header(frame, areas[0], app);
    render_conversation(frame, areas[1], app);
    render_status(frame, areas[2], app);
}

fn render_header(frame: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    let mut spans = vec![
        Span::styled(
            " ⚒ Forge ",
            Style::default().fg(Color::Black).bg(FORGE_ORANGE).bold(),
        ),
        Span::raw(" "),
    ];
    if !app.session_id.is_empty() {
        let short: String = app.session_id.chars().take(8).collect();
        spans.push(Span::styled(
            format!("session {short}"),
            Style::default().fg(Color::DarkGray),
        ));
    }
    if app.done {
        spans.push(Span::raw("  "));
        spans.push(Span::styled("done", Style::default().fg(Color::Green)));
    }
    frame.render_widget(Paragraph::new(TextLine::from(spans)), area);
}

fn render_conversation(frame: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    let items: Vec<ListItem> = app
        .lines
        .iter()
        .map(|line| match line {
            Line::Assistant(text) => ListItem::new(TextLine::from(text.as_str())),
            Line::ToolStart { name, args } => ListItem::new(TextLine::from(vec![
                Span::styled("  ↳ ", Style::default().fg(Color::Cyan)),
                Span::styled(name.clone(), Style::default().fg(Color::Cyan).bold()),
                Span::styled(format!("({args})"), Style::default().fg(Color::DarkGray)),
            ])),
            Line::ToolResult { name, ok, summary } => {
                let (mark, color) = if *ok {
                    ("  ✓ ", Color::Green)
                } else {
                    ("  ✗ ", Color::Red)
                };
                ListItem::new(TextLine::from(vec![
                    Span::styled(mark, Style::default().fg(color)),
                    Span::styled(format!("{name}: "), Style::default().fg(color)),
                    Span::raw(summary.clone()),
                ]))
            }
        })
        .collect();

    let block = Block::bordered()
        .title(" Conversation ")
        .border_style(Style::default().fg(Color::DarkGray));
    frame.render_widget(List::new(items).block(block), area);
}

fn render_status(frame: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    let mut lines: Vec<TextLine> = Vec::new();

    match &app.routing {
        Some(r) => lines.push(TextLine::from(vec![
            Span::styled("mesh ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("[{}] ", r.tier),
                Style::default().fg(FORGE_ORANGE).bold(),
            ),
            Span::styled(
                r.model.clone(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  ({})", r.rationale),
                Style::default().fg(Color::DarkGray),
            ),
        ])),
        None => lines.push(TextLine::from(Span::styled(
            "mesh idle",
            Style::default().fg(Color::DarkGray),
        ))),
    }

    lines.push(TextLine::from(vec![
        Span::styled("cost ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("${:.4}", app.cost_usd),
            Style::default().fg(Color::Green).bold(),
        ),
    ]));

    for w in &app.warnings {
        lines.push(TextLine::from(Span::styled(
            format!("⚠ {w}"),
            Style::default().fg(Color::Yellow),
        )));
    }

    if let Some(p) = &app.prompt {
        lines.push(TextLine::from(Span::styled(
            format!("» {p} [y/N]"),
            Style::default().fg(Color::Black).bg(Color::Yellow).bold(),
        )));
    }

    if app.done {
        lines.push(TextLine::from(Span::styled(
            "press q to quit",
            Style::default().fg(Color::DarkGray),
        )));
    }

    let block = Block::bordered()
        .title(" Model Mesh ")
        .border_style(Style::default().fg(FORGE_ORANGE));
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn screen(app: &App) -> String {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|f| render(f, app)).unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    #[test]
    fn routing_panel_shows_model_and_tier() {
        let mut app = App::default();
        app.apply(PresenterEvent::Routing {
            tier: "complex".into(),
            model: "anthropic::claude-opus-4-8".into(),
            rationale: "matched complex signal".into(),
        });
        let text = screen(&app);
        assert!(
            text.contains("claude-opus-4-8"),
            "missing model in:\n{text}"
        );
        assert!(text.contains("complex"), "missing tier in:\n{text}");
    }

    #[test]
    fn cost_meter_shows_running_total() {
        let mut app = App::default();
        app.apply(PresenterEvent::Cost {
            session_total_usd: 0.0033,
        });
        assert!(screen(&app).contains("$0.0033"));
    }

    #[test]
    fn assistant_text_appears_in_conversation() {
        let mut app = App::default();
        app.apply(PresenterEvent::AssistantText(
            "the workspace looks healthy".into(),
        ));
        assert!(screen(&app).contains("the workspace looks healthy"));
    }

    #[test]
    fn tool_invocation_appears_in_conversation() {
        let mut app = App::default();
        app.apply(PresenterEvent::ToolStart {
            name: "read_file".into(),
            args: "{\"path\":\"Cargo.toml\"}".into(),
        });
        app.apply(PresenterEvent::ToolResult {
            name: "read_file".into(),
            ok: true,
            summary: "[workspace]".into(),
        });
        let text = screen(&app);
        assert!(text.contains("read_file"), "missing tool name in:\n{text}");
    }

    #[test]
    fn budget_warning_is_displayed() {
        let mut app = App::default();
        app.apply(PresenterEvent::Warning(
            "approaching daily budget cap".into(),
        ));
        assert!(screen(&app).contains("approaching daily budget cap"));
    }
}
