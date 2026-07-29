//! UI-loop helpers shared by interactive chat paths.

use super::*;

/// Ingest an external bridge-stat snapshot into the session's shared quota store.
pub(crate) fn seed_subscription_stats(session: &Session, bstats: &bridge_stats::BridgeStats) {
    for (provider, window, fraction, observed_at) in [
        (
            "codex-cli",
            "five_hour",
            bstats.codex_5h_pct,
            bstats.codex_5h_observed_at,
        ),
        (
            "codex-cli",
            "weekly",
            bstats.codex_weekly_pct,
            bstats.codex_weekly_observed_at,
        ),
        (
            "claude-cli",
            "five_hour",
            bstats.claude_5h_pct,
            bstats.claude_5h_observed_at,
        ),
        (
            "claude-cli",
            "weekly",
            bstats.claude_weekly_pct,
            bstats.claude_weekly_observed_at,
        ),
    ] {
        session.seed_subscription_quota_at(provider, window, fraction, observed_at);
    }
}

/// Populate the usage overlay from the shared, normalized quota store.
pub(crate) fn fill_subscription_pcts(
    overlay: &mut forge_tui::UsageOverlay,
    fractions: &std::collections::HashMap<String, std::collections::HashMap<String, f64>>,
    claude_store_age_secs: Option<i64>,
) {
    let stored = |provider: &str, window: &str| {
        fractions
            .get(provider)
            .and_then(|windows| windows.get(window))
            .copied()
    };
    overlay.claude_5h_pct = stored("claude-cli", "five_hour").map(|fraction| fraction * 100.0);
    overlay.claude_weekly_pct = stored("claude-cli", "weekly").map(|fraction| fraction * 100.0);
    overlay.codex_5h_pct = stored("codex-cli", "five_hour").map(|fraction| fraction * 100.0);
    overlay.codex_weekly_pct = stored("codex-cli", "weekly").map(|fraction| fraction * 100.0);
    overlay.claude_rl_age_secs = claude_store_age_secs.filter(|&age| age > 300);
}

/// Synchronize the command palette with the slash token containing the cursor.
pub(crate) fn sync_palette_to_slash_token(app: &mut forge_tui::App) {
    let cursor = app.input_cursor.min(app.input.len());
    let token = forge_tui::slash_token_at(&app.input, cursor)
        .filter(|token| cursor >= token.start && cursor <= token.end);
    match token {
        Some(token) if app.palette.open => {
            app.palette.query = token.name;
            app.palette.clamp();
        }
        Some(token) => app.palette.open_with(&token.name),
        None => app.palette.close(),
    }
}

/// Abort a turn and release any UI prompts that would otherwise hold its task mutex.
pub(crate) fn abort_turn_before_quit(
    turn_handle: &mut Option<tokio::task::JoinHandle<()>>,
    pending: &mut Option<(String, std::sync::mpsc::Sender<forge_tui::ConfirmOutcome>)>,
    pending_question: &mut Option<std::sync::mpsc::Sender<String>>,
    app: &mut forge_tui::App,
) {
    if let Some(handle) = turn_handle.take() {
        handle.abort();
    }
    *pending = None;
    *pending_question = None;
    app.prompt = None;
    app.clear_question();
}

/// Whether `prefix` is a provider Forge recognizes.
pub(crate) fn is_known_provider_prefix(prefix: &str) -> bool {
    forge_config::is_known_provider(prefix)
}

/// The OS shell used to run a custom statusline command.
pub(crate) fn shell_widget_shell() -> (&'static str, &'static str) {
    #[cfg(windows)]
    return ("cmd", "/C");
    #[cfg(not(windows))]
    ("sh", "-c")
}

/// Emit styled lines to native scrollback when available, otherwise to the app transcript.
pub(crate) fn emit_scrollback(
    tui: Option<&mut forge_tui::Tui>,
    app: &mut forge_tui::App,
    lines: Vec<forge_tui::ScrollbackLine<'static>>,
) {
    match tui {
        Some(tui) if !tui.is_fullscreen() => tui.insert_lines(lines),
        _ => app.push_scrollback(lines),
    }
}

/// Emit plain text to native scrollback when available, otherwise to the app transcript.
pub(crate) fn emit_text(tui: Option<&mut forge_tui::Tui>, app: &mut forge_tui::App, text: &str) {
    match tui {
        Some(tui) if !tui.is_fullscreen() => tui.print_text(text),
        _ => app.push_scrollback_text(text),
    }
}

/// Every editable setting as `/config` editor rows.
pub(crate) fn config_editor_rows() -> Vec<forge_tui::SettingRow> {
    let mut rows: Vec<forge_tui::SettingRow> = forge_config::known_key_providers()
        .map(|provider| forge_tui::SettingRow {
            path: format!("key.{provider}"),
            group: "Providers & Keys".to_string(),
            label: format!("{} API key", provider_label(provider)),
            help: Some(format!(
                "API key for {provider}, stored in the OS keyring. Enter to set; empty to remove."
            )),
            kind: forge_tui::RowKind::Secret,
            value: if forge_config::has_api_key(provider) {
                "● set"
            } else {
                "○ not set"
            }
            .to_string(),
            default: String::new(),
            modified: forge_config::has_api_key(provider),
            source: "keyring".to_string(),
        })
        .collect();
    rows.extend(
        forge_config::config_descriptors()
            .into_iter()
            .map(|descriptor| {
                let kind = match descriptor.kind {
                    forge_config::SettingKind::Bool => forge_tui::RowKind::Bool,
                    forge_config::SettingKind::Int => forge_tui::RowKind::Int,
                    forge_config::SettingKind::Float => forge_tui::RowKind::Float,
                    forge_config::SettingKind::List
                    | forge_config::SettingKind::Json
                    | forge_config::SettingKind::Text => forge_tui::RowKind::Text,
                    forge_config::SettingKind::Enum(options) => {
                        forge_tui::RowKind::Enum(options.into_iter().map(str::to_string).collect())
                    }
                };
                forge_tui::SettingRow {
                    path: descriptor.path,
                    group: descriptor.group,
                    label: descriptor.label,
                    help: descriptor.help,
                    kind,
                    value: descriptor.value.display(),
                    default: descriptor.default.display(),
                    modified: descriptor.modified,
                    source: descriptor.source.to_string(),
                }
            }),
    );
    rows.extend(
        forge_config::complex_sections()
            .iter()
            .map(|&section| forge_tui::SettingRow {
                path: section.to_string(),
                group: "Advanced (edit in $EDITOR)".to_string(),
                label: section.to_string(),
                help: Some(forge_config::complex_section_help(section).to_string()),
                kind: forge_tui::RowKind::ReadOnly,
                value: String::new(),
                default: String::new(),
                modified: false,
                source: "config.toml".to_string(),
            }),
    );
    rows
}
