use super::*;
use crate::theme::styles;

/// Height reserved for the active response state.  Questions use the composer for their answer,
/// so they need only a single instruction row; permissions keep their decision keys on row two.
pub(crate) fn prompt_height(app: &App) -> u16 {
    if app.prompt.is_none() {
        0
    } else if app.awaiting_question() {
        1
    } else {
        PERMISSION_H
    }
}

pub(crate) fn render_permission(frame: &mut Frame, area: Rect, app: &App) {
    let Some(prompt) = &app.prompt else {
        return;
    };

    // `prompt` is shared by two fundamentally different interactions.  An AskUserQuestion uses
    // the composer (number/free-text + Enter), while a tool permission is a single-key decision.
    // Showing y/a/n for a question was not merely noisy: it suggested keys that could never
    // answer it.  Keep the two contracts visibly distinct.
    if app.awaiting_question() {
        let text_width = area.width.saturating_sub(11) as usize;
        let line = TextLine::from(vec![
            Span::styled(" ◉ ANSWER ", styles::pill(styles::brand())),
            Span::styled(truncate(prompt, text_width), styles::warn()),
            Span::styled("  Enter submit", styles::muted()),
        ]);
        frame.render_widget(Paragraph::new(line), Rect { height: 1, ..area });
        return;
    }

    let summary_width = area.width.saturating_sub(15) as usize;
    let summary = TextLine::from(vec![
        Span::styled(" ◉ PERMISSION ", styles::pill(styles::brand())),
        Span::styled(truncate(prompt, summary_width), styles::warn()),
    ]);
    frame.render_widget(Paragraph::new(summary), Rect { height: 1, ..area });

    if area.height > 1 {
        let controls = TextLine::from(vec![
            Span::styled("   [y]es", styles::ok().bold()),
            Span::styled("  [a]lways", styles::ok().bold()),
            Span::styled("  [n]o", styles::err().bold()),
            Span::styled("  Esc cancel", styles::muted()),
        ]);
        frame.render_widget(
            Paragraph::new(controls),
            Rect {
                y: area.y + 1,
                height: 1,
                ..area
            },
        );
    }
}

/// Rows the decision block needs; zero when nothing is waiting for the user.
pub(crate) fn block_height(app: &App) -> u16 {
    prompt_height(app)
}

/// Whether the decision block takes the composer's place while it is open.
pub(crate) fn replaces_composer(_app: &App) -> bool {
    false
}

/// Composer hints while a permission prompt or a question waits. `None` when neither does.
pub(crate) fn hints(app: &App) -> Option<Vec<super::chrome::Hint>> {
    use super::chrome::hint;
    if app.awaiting_question() {
        return Some(vec![
            hint("↑↓", "choose"),
            hint("Enter", "confirm"),
            hint("1-3", "pick"),
            hint("o", "other"),
            hint("n", "add note"),
        ]);
    }
    if app.form.is_some() {
        return Some(vec![
            hint("↑↓", "choose"),
            hint("Enter", "confirm"),
            hint("1-3", "pick"),
            hint("o", "other"),
            hint("Esc", "dismiss"),
        ]);
    }
    if app.prompt.is_some() {
        return Some(vec![
            hint("y", "allow"),
            hint("a", "always"),
            hint("n", "deny"),
            hint("Esc", "cancel"),
            hint("?", "keys"),
        ]);
    }
    None
}
