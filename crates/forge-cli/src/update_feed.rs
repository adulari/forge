//! Where Forge learns what the latest release is: the GitHub releases API, or whatever
//! `FORGE_UPDATE_FEED_URL` points at (an API base such as `http://127.0.0.1:8080`, serving
//! `/repos/Adulari/forge/releases/latest`). The override exists for tests and mirrors.

use std::time::Duration;

use serde::{Deserialize, Serialize};

pub(crate) const REPO: &str = "Adulari/forge";
const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// A published release, as the update surfaces need it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    /// The version without a leading `v` (`2.24.0`).
    pub version: String,
    /// The release body as published (markdown).
    pub notes: String,
    /// The release page.
    pub url: String,
}

/// `FORGE_UPDATE_FEED_URL`, trimmed of a trailing slash, when set and non-empty.
pub(crate) fn feed_override() -> Option<String> {
    std::env::var("FORGE_UPDATE_FEED_URL")
        .ok()
        .map(|u| u.trim().trim_end_matches('/').to_string())
        .filter(|u| !u.is_empty())
}

fn latest_url() -> String {
    let base = feed_override().unwrap_or_else(|| "https://api.github.com".to_string());
    format!("{base}/repos/{REPO}/releases/latest")
}

/// The latest published release. Best-effort: any failure (offline, rate limit, bad JSON) is `None`.
pub(crate) async fn fetch_latest() -> Option<Release> {
    let resp = forge_provider::bundled_http_client()
        .get(latest_url())
        .header("User-Agent", format!("forge/{CURRENT}"))
        .header("Accept", "application/vnd.github+json")
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    parse_release(&resp.json().await.ok()?)
}

pub(crate) fn parse_release(v: &serde_json::Value) -> Option<Release> {
    let tag = v.get("tag_name")?.as_str()?;
    let version = tag.trim().trim_start_matches('v').to_string();
    if version.is_empty() {
        return None;
    }
    let text = |key: &str| {
        v.get(key)
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string()
    };
    let url = match text("html_url") {
        u if u.is_empty() => format!("https://github.com/{REPO}/releases/tag/{tag}"),
        u => u,
    };
    Some(Release {
        version,
        notes: text("body"),
        url,
    })
}

/// The first `max` meaningful lines of a release body, as display text: markdown headings lose
/// their `#`, bullets their marker, emphasis its asterisks; blank lines, rules, tables and
/// HTML comments are skipped; each row is cut to `max_chars`.
pub fn notes_preview(body: &str, max: usize, max_chars: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_fence = false;
    for raw in body.lines() {
        let line = raw.trim();
        if line.starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence
            || line.is_empty()
            || line.starts_with('|')
            || line.starts_with("<!--")
            || line.chars().all(|c| matches!(c, '-' | '=' | '_' | '*'))
        {
            continue;
        }
        let line = line.trim_start_matches('#').trim_start();
        let line = line
            .strip_prefix("- ")
            .or_else(|| line.strip_prefix("* "))
            .unwrap_or(line);
        let line = line.replace("**", "").replace('`', "");
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut cut: String = line.chars().take(max_chars).collect();
        if line.chars().count() > max_chars {
            cut.push('…');
        }
        out.push(cut);
        if out.len() == max {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_release_keeps_version_notes_and_page() {
        let r = parse_release(&json!({
            "tag_name": "v2.24.0",
            "body": "## What's new\n- Faster",
            "html_url": "https://example.test/r/v2.24.0",
        }))
        .unwrap();
        assert_eq!(r.version, "2.24.0");
        assert_eq!(r.url, "https://example.test/r/v2.24.0");
        assert!(r.notes.contains("Faster"));
    }

    #[test]
    fn a_missing_page_falls_back_to_the_tag_url_and_garbage_is_refused() {
        let r = parse_release(&json!({ "tag_name": "v1.2.3" })).unwrap();
        assert_eq!(
            r.url,
            "https://github.com/Adulari/forge/releases/tag/v1.2.3"
        );
        assert!(r.notes.is_empty());
        assert!(parse_release(&json!({ "name": "no tag" })).is_none());
        assert!(parse_release(&json!({ "tag_name": "v" })).is_none());
    }

    #[test]
    fn the_preview_is_plain_text_and_capped() {
        let body = "## What's changed\n\n- **TUI**: an update card\n- Fix `resume` picker\n---\n| a | b |\n```\ncode\n```\n* last one that is cut short because it is very long indeed";
        let rows = notes_preview(body, 4, 24);
        assert_eq!(rows[0], "What's changed");
        assert_eq!(rows[1], "TUI: an update card");
        assert_eq!(rows[2], "Fix resume picker");
        assert!(
            rows[3].ends_with('…') && rows[3].chars().count() == 25,
            "{rows:?}"
        );
        assert_eq!(notes_preview("", 3, 40), Vec::<String>::new());
    }
}
