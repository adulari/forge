//! Whether a pinned model id deserves the "unknown model" warning.
//!
//! The cached catalog is only as complete as the process that wrote it. A headless daemon whose
//! `PATH` lacked `~/.local/bin` cataloged no `claude-cli::` models, and every later login-shell
//! run then flagged a perfectly valid `claude-cli::claude-opus-5-5` pin against that stale file.
//! A CLI bridge that is routable in THIS process is therefore judged by the bridge's own alias
//! table, not by catalog membership.

use forge_provider::CliKind;

/// What to do about one pinned id given the cached catalog.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum PinVerdict {
    Ok,
    /// Warn: not in the catalog and not explainable by a routable bridge.
    Unknown,
}

/// True when `model` (the part after `provider::`) looks like a full model name for `kind`'s
/// vendor, which the CLI accepts even though the alias table predates it.
fn plausible_full_id(kind: CliKind, model: &str) -> bool {
    let stem = match kind {
        CliKind::ClaudeCode => "claude-",
        CliKind::Codex => "gpt-",
        CliKind::Antigravity => "gemini-",
    };
    model.starts_with(stem)
}

pub(super) fn judge_pin(
    id: &str,
    prefix: &str,
    in_catalog: bool,
    bridge_routable: impl Fn(CliKind) -> bool,
) -> PinVerdict {
    if in_catalog {
        return PinVerdict::Ok;
    }
    let Some(kind) = CliKind::all().into_iter().find(|k| k.prefix() == prefix) else {
        return PinVerdict::Unknown;
    };
    if !bridge_routable(kind) {
        return PinVerdict::Unknown;
    }
    let model = id.split_once("::").map_or("", |(_, m)| m);
    if model.is_empty() || kind.default_models().contains(&model) || plausible_full_id(kind, model)
    {
        PinVerdict::Ok
    } else {
        PinVerdict::Unknown
    }
}

/// True when the cached catalog has no entry for a bridge that is routable here — the cache was
/// written by a process that could not see that CLI, so it should be refreshed.
pub(super) fn catalog_missing_routable_bridge(
    catalog_models: &[String],
    bridge_routable: impl Fn(CliKind) -> bool,
) -> bool {
    CliKind::all().into_iter().any(|k| {
        bridge_routable(k)
            && !catalog_models
                .iter()
                .any(|m| m.starts_with(&format!("{}::", k.prefix())))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const YES: fn(CliKind) -> bool = |_| true;
    const NO: fn(CliKind) -> bool = |_| false;

    #[test]
    fn catalog_member_is_always_ok() {
        assert_eq!(
            judge_pin("claude-cli::x", "claude-cli", true, NO),
            PinVerdict::Ok
        );
    }

    #[test]
    fn routable_bridge_accepts_full_ids_and_aliases_missing_from_a_stale_catalog() {
        assert_eq!(
            judge_pin("claude-cli::claude-opus-5-5", "claude-cli", false, YES),
            PinVerdict::Ok
        );
        assert_eq!(
            judge_pin("claude-cli::opus", "claude-cli", false, YES),
            PinVerdict::Ok
        );
        assert_eq!(
            judge_pin("claude-cli::", "claude-cli", false, YES),
            PinVerdict::Ok
        );
    }

    #[test]
    fn routable_bridge_still_flags_a_typo() {
        assert_eq!(
            judge_pin("claude-cli::opuss", "claude-cli", false, YES),
            PinVerdict::Unknown
        );
    }

    #[test]
    fn unroutable_bridge_and_other_providers_keep_the_warning() {
        assert_eq!(
            judge_pin("claude-cli::claude-opus-5-5", "claude-cli", false, NO),
            PinVerdict::Unknown
        );
        assert_eq!(
            judge_pin("groq::nope", "groq", false, YES),
            PinVerdict::Unknown
        );
    }

    #[test]
    fn stale_catalog_detection() {
        let only_groq = vec!["groq::a".to_string()];
        assert!(catalog_missing_routable_bridge(&only_groq, |k| k == CliKind::ClaudeCode));
        assert!(!catalog_missing_routable_bridge(&only_groq, NO));
        let with_claude = vec!["claude-cli::opus".to_string()];
        assert!(!catalog_missing_routable_bridge(&with_claude, |k| k == CliKind::ClaudeCode));
    }
}
