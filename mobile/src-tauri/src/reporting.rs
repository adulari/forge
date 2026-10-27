//! Automatic issue reporting, Rust half. A panic hook writes a scrubbed, content-free report into
//! `<data dir>/reports-queue/`; the webview drains the directory at the next launch and sends it
//! through the same sender (and ask-mode card) as JS errors. Nothing is sent from the hook itself,
//! so a crashing process never blocks on the network. The user's choice lives in
//! `<data dir>/reporting-mode`, written by the webview; `FORGE_NO_REPORTS=1` forces off.

use std::path::PathBuf;

use serde::Serialize;

const MAX_QUEUED: usize = 10;
const MAX_MESSAGE: usize = 300;

fn data_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("dev", "forge", "forge")
        .map(|dirs| dirs.data_dir().join("desktop-reporting"))
}

fn env_disabled() -> bool {
    matches!(
        std::env::var("FORGE_NO_REPORTS")
            .ok()
            .as_deref()
            .map(str::trim),
        Some("1" | "true" | "yes" | "on")
    )
}

fn read_mode() -> String {
    let stored = data_dir()
        .and_then(|dir| std::fs::read_to_string(dir.join("reporting-mode")).ok())
        .map(|raw| raw.trim().to_string());
    match stored.as_deref() {
        Some(mode @ ("auto" | "ask" | "off")) => mode.to_string(),
        _ => "auto".to_string(),
    }
}

fn effective_mode() -> String {
    if env_disabled() {
        "off".to_string()
    } else {
        read_mode()
    }
}

fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// Cut absolute paths to a file name, mask credential-shaped tokens, collapse whitespace, truncate.
pub(crate) fn scrub(text: &str) -> String {
    let words: Vec<String> = text
        .split_whitespace()
        .map(|word| {
            let trimmed =
                word.trim_matches(|c: char| matches!(c, '"' | '\'' | '(' | ')' | ',' | ';'));
            if trimmed.contains("://") {
                let host = trimmed
                    .split("://")
                    .nth(1)
                    .and_then(|rest| rest.split(['/', '?', '#']).next())
                    .and_then(|authority| authority.rsplit('@').next())
                    .unwrap_or("");
                return format!("<url:{host}>");
            }
            if (trimmed.starts_with('/') && trimmed.matches('/').count() >= 2)
                || trimmed.contains(":\\")
                || trimmed.starts_with("~/")
            {
                return file_name(trimmed).to_string();
            }
            if trimmed.contains('@') && trimmed.contains('.') {
                return "[email]".to_string();
            }
            let long_opaque = trimmed.len() >= 24
                && trimmed
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '+' | '/' | '='))
                && trimmed.chars().any(|c| c.is_ascii_digit());
            let prefixed = ["sk-", "ghp_", "gho_", "github_pat_", "xox", "AKIA", "eyJ"]
                .iter()
                .any(|p| trimmed.starts_with(p) && trimmed.len() >= 12);
            if long_opaque || prefixed {
                return "[redacted]".to_string();
            }
            word.to_string()
        })
        .collect();
    let joined = words.join(" ");
    if joined.chars().count() > MAX_MESSAGE {
        let cut: String = joined.chars().take(MAX_MESSAGE - 1).collect();
        format!("{cut}…")
    } else {
        joined
    }
}

fn hash16(input: &str) -> String {
    let (mut a, mut b) = (0xcbf2_9ce4_8422_2325_u64, 0x8422_2325_cbf2_9ce4_u64);
    for byte in input.bytes() {
        a = (a ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
        b = (b ^ u64::from(byte))
            .wrapping_mul(0x0000_0100_0000_01b3)
            .rotate_left(5);
    }
    format!(
        "{:08x}{:08x}",
        (a >> 32) as u32 ^ a as u32,
        (b >> 32) as u32 ^ b as u32
    )
}

#[derive(Serialize)]
struct QueuedPanic {
    error_kind: &'static str,
    site: String,
    location: String,
    message: String,
    fingerprint: String,
    occurred_at: u64,
}

fn queue_panic(info: &std::panic::PanicHookInfo<'_>) {
    if effective_mode() == "off" {
        return;
    }
    let Some(dir) = data_dir().map(|d| d.join("reports-queue")) else {
        return;
    };
    let location = info
        .location()
        .map(|l| format!("{}:{}", file_name(l.file()), l.line()))
        .unwrap_or_default();
    let message = info
        .payload()
        .downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| info.payload().downcast_ref::<String>().cloned())
        .unwrap_or_default();
    let site = format!(
        "tauri.panic.{}",
        location.split(':').next().unwrap_or("unknown")
    );
    let report = QueuedPanic {
        error_kind: "tauri_panic",
        fingerprint: hash16(&format!("tauri_panic|{site}")),
        site,
        location,
        message: scrub(&message),
        occurred_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()),
    };
    let _ = std::fs::create_dir_all(&dir);
    let existing = std::fs::read_dir(&dir).map_or(0, Iterator::count);
    if existing >= MAX_QUEUED {
        return;
    }
    if let Ok(json) = serde_json::to_string(&report) {
        let _ = std::fs::write(
            dir.join(format!("{}-{existing}.json", report.occurred_at)),
            json,
        );
    }
}

/// Chain a panic hook that queues a scrubbed report, then runs the previous hook.
pub(crate) fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        queue_panic(info);
        previous(info);
    }));
}

#[derive(Serialize)]
pub(crate) struct ReportingEnv {
    url: Option<String>,
    disabled: bool,
}

#[tauri::command]
pub(crate) fn reporting_env() -> ReportingEnv {
    ReportingEnv {
        url: std::env::var("FORGE_REPORTING_URL")
            .ok()
            .filter(|u| !u.trim().is_empty()),
        disabled: env_disabled(),
    }
}

#[tauri::command]
pub(crate) fn get_reporting_mode() -> String {
    effective_mode()
}

#[tauri::command]
pub(crate) fn set_reporting_mode(mode: String) -> Result<(), String> {
    if !matches!(mode.as_str(), "auto" | "ask" | "off") {
        return Err("unknown reporting mode".to_string());
    }
    let dir = data_dir().ok_or_else(|| "data directory unavailable".to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("reporting-mode"), &mode).map_err(|e| e.to_string())?;
    if mode == "off" {
        let _ = std::fs::remove_dir_all(dir.join("reports-queue"));
    }
    Ok(())
}

/// Returns each queued panic report's JSON and deletes the files.
#[tauri::command]
pub(crate) fn take_queued_reports() -> Vec<String> {
    let Some(dir) = data_dir().map(|d| d.join("reports-queue")) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten().take(MAX_QUEUED) {
        let path = entry.path();
        if let Ok(raw) = std::fs::read_to_string(&path) {
            out.push(raw);
        }
        let _ = std::fs::remove_file(&path);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrub_removes_paths_urls_emails_and_secrets() {
        let out = scrub(
            "failed reading /home/alice/project/src/main.rs via https://user:pw@api.example.com/v1?k=1 mail bob@example.com key sk-abcdefghijklmnop1234 tok 0123456789abcdef0123456789abcdef",
        );
        assert!(out.contains("main.rs"));
        assert!(!out.contains("alice") && !out.contains("/home"));
        assert!(out.contains("<url:api.example.com>"));
        assert!(!out.contains("pw@") && !out.contains("bob@"));
        assert!(!out.contains("sk-abc") && !out.contains("0123456789abcdef"));
    }

    #[test]
    fn scrub_truncates() {
        assert!(scrub(&"word ".repeat(200)).chars().count() <= MAX_MESSAGE);
    }

    #[test]
    fn fingerprint_is_stable_and_16_hex() {
        let f = hash16("tauri_panic|tauri.panic.lib.rs");
        assert_eq!(f.len(), 16);
        assert_eq!(f, hash16("tauri_panic|tauri.panic.lib.rs"));
    }
}
