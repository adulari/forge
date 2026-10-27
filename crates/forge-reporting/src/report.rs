//! The wire format (v1) and the builder that fills it from scrubbed parts.
//!
//! The struct is closed: every field is either an enum, a bounded identifier or text that has been
//! through [`crate::scrub`]. There is no free-form map, so nothing can be attached by accident.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::ring::EventEntry;
use crate::scrub;

pub const WIRE_VERSION: u32 = 1;
pub const MAX_MESSAGE: usize = 300;
pub const MAX_NOTE: usize = 2000;
pub const MAX_LAST_ERROR: usize = 1000;
pub const MANUAL_FINGERPRINT: &str = "0000000000000000";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Auto,
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    Panic,
    Internal,
    ProviderProtocol,
    ToolDispatch,
    Store,
    Compaction,
    TaskDeath,
    RenderPanic,
    JsError,
    JsUnhandledRejection,
    ReactBoundary,
    TauriCommand,
    TauriPanic,
    Manual,
}

impl ErrorKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Panic => "panic",
            Self::Internal => "internal",
            Self::ProviderProtocol => "provider_protocol",
            Self::ToolDispatch => "tool_dispatch",
            Self::Store => "store",
            Self::Compaction => "compaction",
            Self::TaskDeath => "task_death",
            Self::RenderPanic => "render_panic",
            Self::JsError => "js_error",
            Self::JsUnhandledRejection => "js_unhandled_rejection",
            Self::ReactBoundary => "react_boundary",
            Self::TauriCommand => "tauri_command",
            Self::TauriPanic => "tauri_panic",
            Self::Manual => "manual",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Surface {
    Tui,
    Headless,
    Daemon,
    Cli,
    Desktop,
    Ios,
    Android,
    Web,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Context {
    pub version: String,
    pub git_sha: String,
    pub surface: Surface,
    pub os: String,
    pub arch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temper: Option<String>,
    #[serde(default)]
    pub flags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    pub v: u32,
    pub source: Source,
    pub install_id: String,
    pub fingerprint: String,
    pub error_kind: ErrorKind,
    pub site: String,
    pub location: String,
    pub message: String,
    pub frames: Vec<String>,
    pub context: Context,
    pub events: Vec<EventEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub occurred_at: u64,
}

impl Report {
    /// The exact bytes a send would put on the wire, pretty-printed for the "what gets sent" views.
    pub fn preview(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    pub fn is_manual(&self) -> bool {
        self.source == Source::Manual
    }

    /// One line for a card or a status mark.
    pub fn headline(&self) -> String {
        if self.location.is_empty() {
            format!("{} in {}", self.error_kind.as_str(), self.site)
        } else {
            format!("{} at {}", self.error_kind.as_str(), self.location)
        }
    }
}

/// Keep identifiers to the characters the relay accepts.
pub(crate) fn slug(raw: &str, max: usize, extra: &str) -> String {
    raw.chars()
        .filter(|c| c.is_ascii_alphanumeric() || extra.contains(*c))
        .take(max)
        .collect()
}

/// Stable id for "the same problem": error kind, logical site and the top in-crate frames. The
/// message text never takes part (it can carry user data and varies per occurrence), except as a
/// digit-free, content-free shape when there is neither a frame nor a better site to go on.
pub fn fingerprint(kind: ErrorKind, site: &str, frames: &[String], shape: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"forge-report-v1|");
    hasher.update(kind.as_str().as_bytes());
    hasher.update(b"|");
    hasher.update(site.as_bytes());
    for frame in frames.iter().take(5) {
        hasher.update(b">");
        hasher.update(frame.as_bytes());
    }
    if frames.is_empty() {
        hasher.update(b"|");
        hasher.update(message_shape(shape).as_bytes());
    }
    let digest = hasher.finalize();
    digest[..8].iter().map(|b| format!("{b:02x}")).collect()
}

/// Message with content, digits and addresses removed, for fingerprinting only.
fn message_shape(message: &str) -> String {
    let stripped = scrub::strip_content(message);
    let shape: String = stripped
        .chars()
        .map(|c| if c.is_ascii_digit() { '#' } else { c })
        .take(80)
        .collect();
    shape.to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_ignores_message_text_when_frames_exist() {
        let frames = vec!["forge_core::session::run_turn".to_string()];
        let a = fingerprint(
            ErrorKind::Internal,
            "forge_core::turn",
            &frames,
            "path /a/b broke",
        );
        let b = fingerprint(
            ErrorKind::Internal,
            "forge_core::turn",
            &frames,
            "path /c/d broke",
        );
        assert_eq!(a, b);
        assert_eq!(a.len(), 16);
    }

    #[test]
    fn fingerprint_without_frames_uses_a_digit_and_content_free_shape() {
        let a = fingerprint(
            ErrorKind::Panic,
            "a.rs",
            &[],
            "index 12 out of range for \"alice\"",
        );
        let b = fingerprint(
            ErrorKind::Panic,
            "a.rs",
            &[],
            "index 97 out of range for \"bob\"",
        );
        assert_eq!(a, b);
        let c = fingerprint(
            ErrorKind::Panic,
            "b.rs",
            &[],
            "index 12 out of range for \"alice\"",
        );
        assert_ne!(a, c);
    }

    #[test]
    fn kind_changes_the_fingerprint() {
        let f = vec!["x::y".to_string()];
        assert_ne!(
            fingerprint(ErrorKind::Store, "s", &f, ""),
            fingerprint(ErrorKind::Internal, "s", &f, "")
        );
    }
}
