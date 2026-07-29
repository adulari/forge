//! Queue stream decoding, workspace, and notification support.

use anyhow::{Context, Result};

/// Folded view of one child run's NDJSON stream.
#[derive(Default)]
pub(super) struct StreamRun {
    pub(super) session_id: Option<String>,
    pub(super) cost_usd: Option<f64>,
    pub(super) result: Option<String>,
    /// The routed model — last `routing` event wins, so mid-run failover reports the model
    /// that actually finished the work.
    pub(super) model: Option<String>,
}

impl StreamRun {
    pub(super) fn fold_line(&mut self, line: &str) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            return;
        };
        match (
            v.get("type").and_then(|t| t.as_str()),
            v.get("subtype").and_then(|t| t.as_str()),
        ) {
            (Some("system"), Some("init")) => {
                self.session_id = v
                    .get("session_id")
                    .and_then(|s| s.as_str())
                    .map(str::to_string);
            }
            (Some("system"), Some("routing")) => {
                self.model = v.get("model").and_then(|m| m.as_str()).map(str::to_string);
            }
            (Some("system"), Some("usage")) => {
                self.cost_usd = v.get("total_cost_usd").and_then(|c| c.as_f64());
            }
            (Some("result"), _) => {
                self.result = v.get("result").and_then(|r| r.as_str()).map(str::to_string);
            }
            _ => {}
        }
    }
}

/// Lowercase, alnum-preserving, dash-separated slug for the result branch name (≤24 chars).
pub(super) fn slugify(task: &str) -> String {
    let mut slug = String::new();
    let mut last_dash = true;
    for c in task.chars() {
        if slug.len() >= 24 {
            break;
        }
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            slug.push('-');
            last_dash = true;
        }
    }
    let slug = slug.trim_end_matches('-').to_string();
    if slug.is_empty() {
        "task".into()
    } else {
        slug
    }
}

pub(super) fn truncate(s: &str, max: usize) -> &str {
    match s.char_indices().nth(max) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

pub(super) fn status_glyph(status: &str) -> &'static str {
    match status {
        "done" => "✓",
        "empty" => "○",
        "gated" => "⚠",
        "over-budget" => "$",
        "failed" => "✗",
        "running" => "▶",
        _ => "·",
    }
}

pub(super) fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub(super) fn canonical_cwd() -> Result<String> {
    let cwd = std::env::current_dir().context("resolving the current directory")?;
    Ok(cwd
        .canonicalize()
        .unwrap_or(cwd)
        .to_string_lossy()
        .into_owned())
}

pub(super) fn git_repo_root(cwd: &str) -> Result<std::path::PathBuf> {
    let out = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(cwd)
        .output()
        .context("running git")?;
    if !out.status.success() {
        anyhow::bail!("not a git repository");
    }
    Ok(std::path::PathBuf::from(
        String::from_utf8_lossy(&out.stdout).trim(),
    ))
}

/// Fire-and-forget desktop notification so an overnight drain announces itself in the morning.
/// Best-effort on every platform; failures (no DE, no notifier binary) are silently ignored.
pub(super) fn notify_desktop(title: &str, body: &str) {
    if cfg!(target_os = "linux") {
        let _ = std::process::Command::new("notify-send")
            .args([title, body])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    } else if cfg!(target_os = "macos") {
        let script = format!(
            "display notification \"{}\" with title \"{}\"",
            body.replace('"', "'"),
            title.replace('"', "'")
        );
        let _ = std::process::Command::new("osascript")
            .args(["-e", &script])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    } else if cfg!(target_os = "windows") {
        // msg.exe is present on all supported Windows editions; a toast would need a helper
        // module or PowerShell body — this stays dependency-free and disappears on its own.
        let _ = std::process::Command::new("msg")
            .args(["*", "/TIME:30", &format!("{title}: {body}")])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }
}
