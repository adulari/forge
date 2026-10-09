use serde::{Deserialize, Serialize};

/// Session-level tool-safety posture (ADR-0008). Exposed in the UI as the **temper** (the
/// forge/metallurgy framing for the agent's disposition); see `docs/features/temper-modes.md`.
/// Serde accepts both the canonical kebab key and the temper-label alias.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum PermissionMode {
    /// Ask before any side effect. Temper: **Ask**.
    #[default]
    #[serde(alias = "ask")]
    Default,
    /// Auto-allow file writes/edits; still ask for shell. Temper: **Auto-edit**.
    #[serde(alias = "auto-edit", alias = "autoedit")]
    AcceptEdits,
    /// Auto-allow everything (explicit, dangerous opt-in). Temper: **Full**.
    #[serde(alias = "full")]
    Bypass,
    /// Claude-Code-style "auto": proceeds without prompting on safe, reversible actions
    /// (workspace edits, non-destructive shell, network reads) and asks only for risky ones
    /// (destructive shell, writes outside the workspace, network exfil, credential access).
    /// Temper: **Auto**. See `forge_core::permission::auto_risk`.
    Auto,
    /// Read-only: deny all side effects. Temper: **Read-only**.
    #[serde(alias = "read-only", alias = "readonly")]
    Plan,
}

impl PermissionMode {
    /// The temper label shown in the UI — names the permission plainly so the active posture
    /// is obvious at a glance (the dimension is themed "temper"; the values are descriptive).
    pub fn label(self) -> &'static str {
        match self {
            PermissionMode::Plan => "Read-only",
            PermissionMode::Default => "Ask",
            PermissionMode::AcceptEdits => "Auto-edit",
            PermissionMode::Bypass => "Full",
            PermissionMode::Auto => "Auto",
        }
    }

    /// One-line description of what this temper does, for the mode picker.
    pub fn description(self) -> &'static str {
        match self {
            PermissionMode::Plan => "analyze & plan only — no file edits or commands",
            PermissionMode::Default => "ask before every file edit and command",
            PermissionMode::AcceptEdits => "auto-apply file edits; still ask before shell commands",
            PermissionMode::Bypass => "auto-approve everything — dangerous, explicit opt-in",
            PermissionMode::Auto => "proceed on safe actions; ask only for risky ones",
        }
    }

    /// All tempers, safest → most permissive, for the mode picker (unlike the SHIFT+TAB cycle,
    /// the picker can reach `Full`/Bypass since it's an explicit, deliberate choice).
    pub fn all() -> &'static [PermissionMode] {
        &[
            PermissionMode::Plan,
            PermissionMode::Default,
            PermissionMode::AcceptEdits,
            PermissionMode::Auto,
            PermissionMode::Bypass,
        ]
    }

    /// Parse a temper from its UI label (or canonical/kebab key) — used to resolve a picker row.
    pub fn from_label(s: &str) -> Option<PermissionMode> {
        match s.trim().to_lowercase().as_str() {
            "read-only" | "readonly" | "plan" => Some(PermissionMode::Plan),
            "ask" | "default" => Some(PermissionMode::Default),
            "accept-edits" | "auto-edit" | "autoedit" | "acceptedits" => {
                Some(PermissionMode::AcceptEdits)
            }
            "full" | "bypass" => Some(PermissionMode::Bypass),
            "auto" => Some(PermissionMode::Auto),
            _ => None,
        }
    }

    /// Canonical kebab key (matches the serde rename) — stable for crossing a process boundary,
    /// e.g. the `FORGE_PERMISSION_MODE` env the parent hands its CLI-bridge `forge mcp-serve` child
    /// so the bridge gates on the parent's *runtime* temper, not the stale on-disk config mode.
    pub fn key(self) -> &'static str {
        match self {
            PermissionMode::Plan => "plan",
            PermissionMode::Default => "default",
            PermissionMode::AcceptEdits => "accept-edits",
            PermissionMode::Bypass => "bypass",
            PermissionMode::Auto => "auto",
        }
    }

    /// Inverse of [`PermissionMode::key`] — exact, no fuzzy aliases.
    pub fn from_key(s: &str) -> Option<PermissionMode> {
        match s {
            "plan" => Some(PermissionMode::Plan),
            "default" => Some(PermissionMode::Default),
            "accept-edits" => Some(PermissionMode::AcceptEdits),
            "bypass" => Some(PermissionMode::Bypass),
            "auto" => Some(PermissionMode::Auto),
            _ => None,
        }
    }

    /// The next temper in the SHIFT+TAB cycle. The cycle covers the three everyday tempers and
    /// **wraps** — `Bypass`/Full is intentionally excluded (reachable only via explicit
    /// `--mode bypass`/config, never by tapping a key). From Full, cycling re-enters
    /// the safe loop at Ask.
    pub fn cycle_next(self) -> PermissionMode {
        match self {
            PermissionMode::Default => PermissionMode::AcceptEdits, // Ask → Auto-edit
            PermissionMode::AcceptEdits => PermissionMode::Auto,    // Auto-edit → Auto
            PermissionMode::Auto => PermissionMode::Plan,           // Auto → Read-only
            PermissionMode::Plan => PermissionMode::Default,        // Read-only → Ask (wrap)
            PermissionMode::Bypass => PermissionMode::Default,      // leave the unsafe temper
        }
    }
}
