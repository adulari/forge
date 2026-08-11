//! `/refine` command bodies — status listing, rollback resolution, and the id-prefix resolver.
//! Split from dispatch.rs to keep that file within its architecture-size budget.

use std::sync::Arc;

use forge_core::Session;

/// `/refine status` — list harness entries in scope plus recent refinement history.
pub(crate) async fn refine_status(
    session: &Arc<tokio::sync::Mutex<Session>>,
    app: &mut forge_tui::App,
) -> anyhow::Result<()> {
    let (overview, history) = {
        let s = session.lock().await;
        let session_id = s.id().to_string();
        let overview = s.harness_overview().map_err(|e| anyhow::anyhow!("{e}"))?;
        let history = s
            .store
            .harness_refinements(Some(&session_id), 5)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        (overview, history)
    };
    if overview.is_empty() {
        app.note("no harness entries yet");
    } else {
        app.note(&format!(
            "{} harness entr{}:",
            overview.len(),
            if overview.len() == 1 { "y" } else { "ies" }
        ));
        for e in &overview {
            app.note(&format!(
                "  {}  {} {} {:?}",
                &e.id[..e.id.len().min(8)],
                e.scope,
                e.kind,
                e.title
            ));
        }
    }
    if history.is_empty() {
        app.note("no refinements yet");
    } else {
        app.note("recent refinements:");
        for r in &history {
            app.note(&format!(
                "  {}  {} — {}",
                &r.id[..r.id.len().min(8)],
                r.trigger,
                r.summary
            ));
        }
    }
    Ok(())
}

/// `/refine rollback <id>` — resolve the id (exact or unique prefix) and invert that batch.
pub(crate) async fn refine_rollback_cmd(
    session: &Arc<tokio::sync::Mutex<Session>>,
    app: &mut forge_tui::App,
    id_arg: &str,
) -> anyhow::Result<()> {
    let id_arg = id_arg.trim();
    if id_arg.is_empty() {
        app.note("usage: /refine rollback <id>");
        return Ok(());
    }
    let mut s = session.lock().await;
    let session_id = s.id().to_string();
    let candidates = s
        .store
        .harness_refinements(Some(&session_id), 200)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    match resolve_refinement_id(id_arg, &candidates) {
        Ok(id) => match s.refine_rollback(&id) {
            Ok(r) => app.note(&format!(
                "↺ rolled back {}: {} edit(s) reverted",
                &id[..id.len().min(8)],
                r.edits.len()
            )),
            Err(e) => app.note(&format!("⚠ rollback failed: {e}")),
        },
        Err(msg) => app.note(&format!("⚠ {msg}")),
    }
    Ok(())
}

/// Resolve a user-typed `/refine rollback <id>` argument against this session's recent
/// refinements: an exact id always wins; otherwise a prefix is accepted if it names exactly one
/// candidate. `candidates` is the caller's own bounded `harness_refinements` page, so this stays a
/// cheap in-memory scan rather than a new store query shape.
fn resolve_refinement_id(
    id_arg: &str,
    candidates: &[forge_store::HarnessRefinement],
) -> std::result::Result<String, String> {
    if candidates.iter().any(|r| r.id == id_arg) {
        return Ok(id_arg.to_string());
    }
    let matches: Vec<&str> = candidates
        .iter()
        .filter(|r| r.id.starts_with(id_arg))
        .map(|r| r.id.as_str())
        .collect();
    match matches.as_slice() {
        [one] => Ok(one.to_string()),
        [] => Err(format!("no refinement matches id '{id_arg}'")),
        _ => Err(format!(
            "ambiguous id prefix '{id_arg}' — {} refinements match",
            matches.len()
        )),
    }
}

#[cfg(test)]
mod refine_tests {
    use super::resolve_refinement_id;

    fn refinement(id: &str) -> forge_store::HarnessRefinement {
        forge_store::HarnessRefinement {
            id: id.to_string(),
            session_id: "s1".into(),
            trigger: "manual".into(),
            summary: "sum".into(),
            rationale: "why".into(),
            expected_outcome: "outcome".into(),
            edits: Vec::new(),
            created_at: 0,
        }
    }

    #[test]
    fn resolve_refinement_id_exact_match_wins() {
        let candidates = vec![refinement("abc123"), refinement("abc999")];
        assert_eq!(
            resolve_refinement_id("abc123", &candidates).unwrap(),
            "abc123"
        );
    }

    #[test]
    fn resolve_refinement_id_accepts_unique_prefix() {
        let candidates = vec![refinement("abc123"), refinement("def456")];
        assert_eq!(resolve_refinement_id("abc", &candidates).unwrap(), "abc123");
    }

    #[test]
    fn resolve_refinement_id_rejects_ambiguous_prefix() {
        let candidates = vec![refinement("abc123"), refinement("abc999")];
        let err = resolve_refinement_id("abc", &candidates).unwrap_err();
        assert!(err.contains("ambiguous"));
    }

    #[test]
    fn resolve_refinement_id_rejects_unknown_id() {
        let candidates = vec![refinement("abc123")];
        let err = resolve_refinement_id("zzz", &candidates).unwrap_err();
        assert!(err.contains("no refinement matches"));
    }
}
