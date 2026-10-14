//! The parts of an `open_pr` body the tool crate cannot build itself: provenance (from the session
//! store) and secret redaction (from the configured keys). Installed once at startup, so the
//! direct path and a CLI bridge's `forge mcp-serve` child (the same binary) both have them.

use std::sync::Arc;

use crate::cli::commands::replay::open_store;

pub(crate) fn install(config: Option<&forge_config::Config>) {
    forge_tools::set_open_pr_settings(forge_tools::OpenPrSettings {
        publish_images: config.is_none_or(|c| c.pr.publish_evidence_images),
        repo_publish_images: Some(Arc::new(forge_config::pr::repo_publish_evidence_images)),
        provenance: Some(Arc::new(pr_provenance_block)),
        redact: Some(Arc::new(|text: &str| {
            redact_with(&forge_core::redact::Redactor::from_environment(), text)
        })),
    });
}

/// A PR body is public: it goes through the same redaction as replay/export.
fn redact_with(redactor: &forge_core::redact::Redactor, text: &str) -> String {
    redactor.redact(text).into_owned()
}

/// The provenance section: enough for a reviewer to trace every change back to the conversation
/// that produced it. Best-effort — a store read failure just yields a shorter block (the PR must
/// never be blocked on bookkeeping).
fn pr_provenance_block(session_id: &str, diff_stat: Option<&str>) -> String {
    let mut block = String::from("### Provenance\n");
    block.push_str(&format!(
        "- Session: `{session_id}` — inspect with `forge replay {short}`\n",
        short = &session_id[..session_id.len().min(8)]
    ));
    if let Ok(store) = open_store() {
        if let Ok(models) = store.session_models(session_id) {
            let mut distinct: Vec<String> = Vec::new();
            for m in models {
                if !distinct.contains(&m) {
                    distinct.push(m);
                }
            }
            if !distinct.is_empty() {
                block.push_str(&format!("- Models: {}\n", distinct.join(", ")));
            }
        }
        if let Ok(cost) = store.session_cost(session_id) {
            if let Ok((tin, tout)) = store.session_tokens(session_id) {
                block.push_str(&format!("- Spend: ${cost:.4} (↑{tin} ↓{tout} tokens)\n"));
            }
        }
    }
    if let Some(diff) = diff_stat {
        block.push_str(&format!("- Diff: {diff}\n"));
    }
    block.push_str("\n🔨 Created with Forge\n");
    block
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provenance_names_the_session_and_the_diff_when_known() {
        let block =
            pr_provenance_block("0123456789abcdef", Some("2 files changed, 9 insertions(+)"));
        assert!(block.starts_with("### Provenance\n- Session: `0123456789abcdef` — inspect with `forge replay 01234567`\n"));
        assert!(block.contains("- Diff: 2 files changed, 9 insertions(+)\n"));
        assert!(block.ends_with("\n🔨 Created with Forge\n"));
        assert!(!pr_provenance_block("s", None).contains("- Diff:"));
    }

    #[test]
    fn redaction_masks_known_secrets_before_the_body_is_public() {
        let redactor = forge_core::redact::Redactor::new(["planted-secret-value-1234".to_string()]);
        let redacted = redact_with(&redactor, "pytest --token planted-secret-value-1234");
        assert!(!redacted.contains("planted-secret-value-1234"));
        assert!(redacted.contains(forge_core::redact::REDACTED));
    }
}
