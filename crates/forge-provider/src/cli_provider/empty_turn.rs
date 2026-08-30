//! Detecting a bridged CLI turn that succeeded while producing nothing.
//!
//! A bridge can exit 0 — or report a clean `result` on the persistent path — with genuinely
//! nothing to show: no assistant text, no tool it ran, and no tool call recovered from prose. The
//! nonzero-exit checks never fire, so this used to become a "successful" empty `ModelResponse`:
//! the run loop had nothing to act on or display, and the session went idle with no error visible
//! to the CLI, the TUI, or the phone.
//!
//! Observed live in the quota-stall incident (2026-08-30, docs/features/mesh-routing.md §5.3.1):
//! codex-cli at a 0.97 five-hour window exited cleanly with empty stdout instead of erroring. An
//! empty result must never look like a completed turn, so both turn paths fail instead.

use forge_types::{QuotaHint, QuotaStatus};

use crate::ProviderError;

/// Build the error for a turn that completed without producing anything. `phrase` describes how
/// the turn ended, since the one-shot and persistent paths reach this from different signals.
pub(super) fn error(binary: &str, phrase: &str, quotas: &[QuotaHint]) -> ProviderError {
    ProviderError::Request(format!("`{binary}` {phrase}{}", quota_note(quotas)))
}

/// Best-effort explanation for an empty-but-successful bridge exit: the most pressured window
/// observed THIS turn, if any. `""` when nothing was pressured — an empty turn can have other
/// causes (a genuinely blank reply, a CLI update), so this only adds detail, never invents a
/// cause.
fn quota_note(quotas: &[QuotaHint]) -> String {
    let worst = quotas
        .iter()
        .filter(|q| q.status != QuotaStatus::Ok)
        .max_by(|a, b| {
            a.fraction_used
                .unwrap_or(0.0)
                .total_cmp(&b.fraction_used.unwrap_or(0.0))
        });
    let Some(q) = worst else {
        return String::new();
    };
    let pct = q
        .fraction_used
        .map(|f| format!(" at {:.0}%", f * 100.0))
        .unwrap_or_default();
    let reset = q
        .resets_at
        .map(|t| {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;
            format!(", resets in {}m", (t - now).max(0) / 60)
        })
        .unwrap_or_default();
    format!(
        " — likely cause: {} {} window is {:?}{pct}{reset}",
        q.provider, q.window, q.status
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hint(window: &str, status: QuotaStatus, fraction: Option<f64>) -> QuotaHint {
        QuotaHint {
            provider: "codex-cli".into(),
            window: window.into(),
            status,
            resets_at: None,
            fraction_used: fraction,
        }
    }

    #[test]
    fn no_pressured_window_invents_no_cause() {
        assert_eq!(quota_note(&[]), "");
        assert_eq!(
            quota_note(&[hint("five_hour", QuotaStatus::Ok, Some(0.1))]),
            ""
        );
    }

    #[test]
    fn names_the_most_pressured_window() {
        let note = quota_note(&[
            hint("weekly", QuotaStatus::Warning, Some(0.81)),
            hint("five_hour", QuotaStatus::Exhausted, Some(0.97)),
        ]);
        assert!(note.contains("five_hour"), "{note}");
        assert!(note.contains("97%"), "{note}");
    }

    #[test]
    fn error_carries_the_binary_and_phrase() {
        let err = error("codex", "exited 0 but produced no assistant content", &[]);
        assert!(
            matches!(err, ProviderError::Request(ref m)
                if m.contains("codex") && m.contains("no assistant content")),
            "{err:?}"
        );
    }
}
