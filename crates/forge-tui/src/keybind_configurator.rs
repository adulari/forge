//! Interactive keybind configurator overlay. Runs as a fullscreen takeover (via `Tui::run_fullscreen`).
//! The user can navigate binds, rebind individual actions, reset to defaults, and save.

use crate::theme::styles;
use std::io;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use ratatui::Terminal;

use forge_config::KeybindsConfig;

use crate::app::chrome_hint as hint;
use crate::app::list_view;
use crate::keybinds::{combo_display, key_event_to_combo};
use crate::surface::{self};

/// Description for each action.
pub(crate) fn action_desc(action: &str) -> &'static str {
    match action {
        "interrupt" => "Stop current turn",
        "command_center" => "Open the searchable command center",
        "command_palette" => "Open slash-command palette",
        "skip_model" => "Mid-turn: abort + retry next model",
        "tier_up" => "Escalate to next tier (mid-turn)",
        "tier_down" => "De-escalate tier (mid-turn)",
        "toggle_reasoning" => "Show/hide reasoning blocks",
        "toggle_tool_groups" => "Open or fold every tool group",
        "prev_tool_group" => "Focus the previous tool group (Enter toggles)",
        "next_tool_group" => "Focus the next tool group (Enter toggles)",
        "undo" => "Undo last file write",
        "compact" => "Compact/summarize conversation",
        "redraw" => "Repaint the screen",
        "model_picker" => "Open model picker overlay",
        "effort_cycle" => "Cycle effort level (low→max)",
        "temper_cycle" => "Cycle temper (default→bypass)",
        "keybind_config" => "Open this keybind configurator",
        "new_session" => "Start a fresh session",
        "copy_last" => "Copy last response to clipboard",
        "scroll_up" => "Scroll transcript up",
        "scroll_down" => "Scroll transcript down",
        "help" => "Show keybind reference (this screen)",
        "checkpoint" => "Save session checkpoint",
        "reload" => "Hot-reload config",
        "voice" => "Open /voice recording overlay",
        "kill_line_forward" => "Delete from cursor to end of line",
        _ => "",
    }
}

/// Whether a binding only applies mid-turn.
fn is_mid_turn_only(action: &str) -> bool {
    matches!(action, "skip_model" | "tier_up" | "tier_down" | "interrupt")
}

/// All actions in display order.
pub(crate) fn all_actions() -> Vec<&'static str> {
    vec![
        "interrupt",
        "command_palette",
        "command_center",
        "skip_model",
        "tier_up",
        "tier_down",
        "toggle_reasoning",
        "toggle_tool_groups",
        "prev_tool_group",
        "next_tool_group",
        "undo",
        "compact",
        "redraw",
        "model_picker",
        "effort_cycle",
        "temper_cycle",
        "keybind_config",
        "new_session",
        "copy_last",
        "scroll_up",
        "scroll_down",
        "help",
        "checkpoint",
        "reload",
        "voice",
        "kill_line_forward",
    ]
}

enum OverlayMode {
    Browse,
    Capture { action: String },
}

/// Run the interactive keybind configurator as a fullscreen takeover.
/// Returns `Ok(true)` if changes were saved, `Ok(false)` if discarded.
pub fn run_keybind_configurator(keybinds: &mut KeybindsConfig) -> io::Result<bool> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    crossterm::execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let defaults = KeybindsConfig::default();
    let mut working = keybinds.clone();
    let actions = all_actions();
    let mut selected: usize = 0;
    let mut mode = OverlayMode::Browse;
    let mut status: Option<(String, bool)> = None;
    let mut changes = 0usize;
    let mut saved = false;

    loop {
        let actions_len = actions.len();
        terminal.draw(|f| {
            let area = f.area();
            let items: Vec<list_view::Item> = actions
                .iter()
                .map(|action| {
                    let combo = working.binds.get(*action);
                    let binding = combo
                        .map(combo_display)
                        .unwrap_or_else(|| "(unset)".to_string());
                    let modified = defaults.binds.get(*action) != working.binds.get(*action);
                    let mut desc = action_desc(action).to_string();
                    if is_mid_turn_only(action) {
                        desc.push_str("  (mid-turn only)");
                    }
                    list_view::Item::Row(list_view::Row {
                        label: if modified {
                            format!("{binding} *")
                        } else {
                            binding
                        },
                        label_style: Some(if modified {
                            styles::warn()
                        } else {
                            styles::fg()
                        }),
                        desc,
                        meta: action.to_string(),
                        ..list_view::Row::default()
                    })
                })
                .collect();
            let detail = match &status {
                Some((msg, ok)) => Some(vec![Span::styled(
                    msg.clone(),
                    if *ok { styles::ok() } else { styles::err() },
                )]),
                None => Some(vec![Span::styled(
                    "* modified from the default",
                    styles::muted(),
                )]),
            };
            let footer = match &mode {
                OverlayMode::Browse => vec![
                    hint("↑↓", "move"),
                    hint("Enter", "rebind"),
                    hint("r", "reset"),
                    hint("R", "reset all"),
                    hint("s", "save"),
                    hint("Esc", "discard"),
                ],
                OverlayMode::Capture { .. } => {
                    vec![hint("any key", "bind it"), hint("Esc", "cancel")]
                }
            };
            list_view::render(
                f,
                area,
                &list_view::View {
                    title: "Keybindings".to_string(),
                    context: if changes > 0 {
                        format!("{changes} unsaved")
                    } else {
                        "no changes".to_string()
                    },
                    items,
                    selected,
                    detail,
                    footer,
                    padded: true,
                    ..list_view::View::default()
                },
            );

            if matches!(mode, OverlayMode::Capture { .. }) {
                let action_name = match &mode {
                    OverlayMode::Capture { action } => action.clone(),
                    _ => String::new(),
                };
                let popup_area = surface::modal_area(area, 44, 5);
                let popup_inner = surface::render_panel(
                    f,
                    popup_area,
                    surface::title("Rebind", surface::SurfaceTone::Brand),
                    None,
                    surface::SurfaceTone::Brand,
                );
                let msg = format!("Press the new key for:\n  {}", action_name);
                f.render_widget(Paragraph::new(Span::styled(msg, styles::fg())), popup_inner);
            }
        })?;

        if let Ok(ev) = event::read() {
            status = None;
            match &mode {
                OverlayMode::Capture { action } => {
                    let action = action.clone();
                    match ev {
                        Event::Key(k) if k.kind == KeyEventKind::Press => {
                            if k.code == KeyCode::Esc {
                                mode = OverlayMode::Browse;
                            } else if let Some(combo) = key_event_to_combo(&k) {
                                let owner = working
                                    .binds
                                    .iter()
                                    .find(|(a, c)| **a != action && **c == combo)
                                    .map(|(a, _)| a.clone());
                                if let Some(owner) = owner {
                                    status = Some((
                                        format!(
                                            "✗ {} is already bound to {} — pick another key",
                                            combo_display(&combo),
                                            owner
                                        ),
                                        false,
                                    ));
                                    mode = OverlayMode::Browse;
                                    continue;
                                }
                                working.binds.insert(action.clone(), combo.clone());
                                changes += 1;
                                status = Some((
                                    format!("✓ {} -> {}", action, combo_display(&combo)),
                                    true,
                                ));
                                mode = OverlayMode::Browse;
                            }
                        }
                        _ => {}
                    }
                }
                OverlayMode::Browse => match ev {
                    Event::Key(k) if k.kind == KeyEventKind::Press => match k.code {
                        KeyCode::Char('q') | KeyCode::Esc => {
                            break;
                        }
                        KeyCode::Char('s') => {
                            *keybinds = working.clone();
                            saved = true;
                            break;
                        }
                        KeyCode::Char('j') | KeyCode::Down => {
                            if selected + 1 < actions_len {
                                selected += 1;
                            }
                        }
                        KeyCode::Char('k') | KeyCode::Up => {
                            selected = selected.saturating_sub(1);
                        }
                        KeyCode::Enter => {
                            let action = actions[selected].to_string();
                            mode = OverlayMode::Capture { action };
                        }
                        // Shift-R (uppercase, with or without the SHIFT modifier flag depending on
                        // the terminal) resets ALL binds; plain 'r' resets only the selected one.
                        KeyCode::Char('R') => {
                            working = defaults.clone();
                            changes += 1;
                            status = Some(("✓ all keybinds reset to defaults".to_string(), true));
                        }
                        KeyCode::Char('r') if k.modifiers.contains(KeyModifiers::SHIFT) => {
                            working = defaults.clone();
                            changes += 1;
                            status = Some(("✓ all keybinds reset to defaults".to_string(), true));
                        }
                        KeyCode::Char('r') => {
                            let action = actions[selected];
                            if let Some(default_combo) = defaults.binds.get(action) {
                                working
                                    .binds
                                    .insert(action.to_string(), default_combo.clone());
                                changes += 1;
                                status = Some((format!("✓ {} reset to default", action), true));
                            }
                        }
                        _ => {}
                    },
                    _ => {}
                },
            }
        }
    }

    crossterm::execute!(io::stdout(), LeaveAlternateScreen)?;
    disable_raw_mode()?;
    Ok(saved)
}
