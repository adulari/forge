//! Small pure helpers behind the daemon terminals in `serve_terminal`: id validation and ordering,
//! geometry clamps and the shell a new terminal starts.

use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) const MAX_TERMINAL_ID_BYTES: usize = 64;
pub(crate) const MAX_COLS: u16 = 1_000;
pub(crate) const MAX_ROWS: u16 = 1_000;

pub(crate) fn terminal_id_order(left: &str, right: &str) -> std::cmp::Ordering {
    fn numeric_suffix(value: &str) -> Option<u64> {
        value.strip_prefix("term-")?.parse().ok()
    }
    match (numeric_suffix(left), numeric_suffix(right)) {
        (Some(left), Some(right)) => left.cmp(&right),
        _ => left.cmp(right),
    }
}

/// Terminal ids that belong to a project action (`[[actions]]`) rather than a plain shell.
pub(crate) const ACTION_TERMINAL_PREFIX: &str = "act-";

pub(crate) fn action_terminal_id(action: &str) -> String {
    format!("{ACTION_TERMINAL_PREFIX}{action}")
}

pub(crate) fn action_of_terminal(terminal_id: &str) -> Option<&str> {
    terminal_id
        .strip_prefix(ACTION_TERMINAL_PREFIX)
        .filter(|action| !action.is_empty())
}

pub(crate) fn valid_terminal_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_TERMINAL_ID_BYTES
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

/// The program and arguments a new terminal runs: `[terminal] shell` / `shell_args`, else the
/// platform's login shell.
pub(crate) fn terminal_shell() -> (String, Vec<String>) {
    let config = forge_config::load().unwrap_or_default().terminal;
    if let Some(shell) = config.shell.filter(|shell| !shell.trim().is_empty()) {
        return (shell, config.shell_args);
    }
    let fallback = if cfg!(windows) {
        std::env::var("COMSPEC").unwrap_or_else(|_| "powershell.exe".to_string())
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
    };
    (fallback, config.shell_args)
}

pub(crate) fn clamp_size(cols: Option<u16>, rows: Option<u16>) -> (u16, u16) {
    (
        cols.unwrap_or(80).clamp(1, MAX_COLS),
        rows.unwrap_or(24).clamp(1, MAX_ROWS),
    )
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}
