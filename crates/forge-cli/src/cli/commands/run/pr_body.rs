//! The deterministic sections `/pr` puts at the end of a PR body. The model is told to paste them
//! verbatim; it fills in neither, so it cannot invent provenance or evidence.

use crate::cli::commands::replay::open_store;

/// Evidence the session captured (when it captured any), then provenance.
pub(super) fn pr_body_sections(session_id: &str, workspace: &std::path::Path) -> String {
    join_sections(
        forge_tools::evidence::pr_evidence_block(workspace, session_id),
        &pr_provenance_block(session_id),
    )
}

fn join_sections(evidence: Option<String>, provenance: &str) -> String {
    match evidence {
        Some(evidence) => format!("---\n{evidence}\n{provenance}"),
        None => format!("---\n{provenance}"),
    }
}

/// The provenance section `/pr` appends to a PR body: enough for a reviewer to trace every change
/// back to the conversation that produced it. Best-effort — a store read failure just yields a
/// shorter block (the PR must never be blocked on bookkeeping).
fn pr_provenance_block(session_id: &str) -> String {
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
    block.push_str("- Diff: <insert `git diff --stat` summary here>\n");
    block.push_str("\n🔨 Created with Forge\n");
    block
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evidence_sits_before_provenance_and_is_omitted_when_empty() {
        let dir = tempfile::tempdir().unwrap();
        let provenance = "### Provenance\n- Session: `sess-1`\n";
        let none = forge_tools::evidence::pr_evidence_block(dir.path(), "sess-1");
        assert_eq!(
            join_sections(none, provenance),
            format!("---\n{provenance}")
        );

        let ledger = forge_tools::evidence::Ledger::new(dir.path(), "sess-1").unwrap();
        ledger.record_test_output("cargo test", 0, "ok").unwrap();
        let evidence = forge_tools::evidence::pr_evidence_block(dir.path(), "sess-1");
        let body = join_sections(evidence, provenance);
        let evidence_at = body.find("### Evidence (captured, not reviewed)").unwrap();
        assert!(body.starts_with("---\n"));
        assert!(evidence_at < body.find("### Provenance").unwrap());
        assert!(body.contains("`cargo test` — exit 0"));
        assert!(body.ends_with(provenance));
    }
}
