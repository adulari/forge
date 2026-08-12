//! `/refine` command bodies — status listing, rollback resolution, and the id-prefix resolver.
//! Split from dispatch.rs to keep that file within its architecture-size budget.

use std::sync::Arc;

use forge_core::Session;

/// `/refine status` — list harness entries in scope plus recent refinement history.
pub(crate) async fn refine_status(
    session: &Arc<tokio::sync::Mutex<Session>>,
    app: &mut forge_tui::App,
) -> anyhow::Result<()> {
    let (overview, history, harness) = {
        let s = session.lock().await;
        let session_id = s.id().to_string();
        let overview = s.harness_overview().map_err(|e| anyhow::anyhow!("{e}"))?;
        let history = s
            .store
            .harness_refinements(Some(&session_id), 5)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        (overview, history, s.harness_config().clone())
    };
    for line in harness_status_lines(&overview, &harness) {
        app.note(&line);
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

/// Render `/refine status`' entry listing: what is stored, grouped by scope, and for each entry
/// whether it actually reaches the model.
///
/// The distinction is the whole point. `Session::harness_overview` returns everything stored,
/// deliberately ignoring `harness.enabled`, while injection additionally caps the list at
/// `max_context_entries` and clamps each entry to `max_entry_chars`
/// (`context_pipeline::harness_context_block`). Listing the raw overview therefore presents
/// entries as active when they are switched off entirely, or when they sit past the cap and are
/// silently dropped from every turn. For a feature whose value rests on being trustworthy, a
/// status view that overstates what is in effect is worse than none.
///
/// Entries arrive in scope-precedence order (session, project, global) and injection caps the
/// *front* of that list, so an entry is injected exactly when its index is under the cap.
fn harness_status_lines(
    entries: &[forge_store::HarnessEntry],
    harness: &forge_config::HarnessConfig,
) -> Vec<String> {
    if entries.is_empty() {
        return vec!["no harness entries yet".to_string()];
    }

    let cap = harness.max_context_entries as usize;
    let injected_count = if harness.enabled {
        entries.len().min(cap)
    } else {
        0
    };

    let mut lines = Vec::new();
    lines.push(if harness.enabled {
        format!(
            "harness: injecting {} of {} entr{} (cap {}, {} chars each)",
            injected_count,
            entries.len(),
            if entries.len() == 1 { "y" } else { "ies" },
            cap,
            harness.max_entry_chars,
        )
    } else {
        format!(
            "harness: OFF — {} entr{} stored, none injected (harness.enabled = false)",
            entries.len(),
            if entries.len() == 1 { "y" } else { "ies" },
        )
    });

    let mut current_scope = "";
    for (index, entry) in entries.iter().enumerate() {
        if entry.scope != current_scope {
            current_scope = &entry.scope;
            lines.push(format!("  {current_scope}"));
        }
        let status = if !harness.enabled {
            "not injected — harness off"
        } else if index >= cap {
            "NOT INJECTED — beyond the entry cap"
        } else if entry.content.chars().count() > harness.max_entry_chars as usize {
            "injected, content truncated"
        } else {
            "injected"
        };
        lines.push(format!(
            "    {}  {:8} {}  [{}]  ({})",
            &entry.id[..entry.id.len().min(8)],
            entry.kind,
            entry.title,
            status,
            entry.source,
        ));
    }
    lines
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
    use super::{harness_status_lines, resolve_refinement_id};

    fn entry(id: &str, scope: &str, content: &str) -> forge_store::HarnessEntry {
        forge_store::HarnessEntry {
            id: id.to_string(),
            scope: scope.to_string(),
            kind: "prompt".into(),
            title: "a lesson".into(),
            content: content.to_string(),
            source: "refinement".into(),
            version: 1,
            created_at: 0,
            updated_at: 0,
        }
    }

    fn harness(enabled: bool, cap: u32, chars: u32) -> forge_config::HarnessConfig {
        forge_config::HarnessConfig {
            enabled,
            auto_refine: forge_config::AutoRefineMode::Off,
            auto_refine_turns: 20,
            max_context_entries: cap,
            max_entry_chars: chars,
        }
    }

    #[test]
    fn harness_status_reports_nothing_stored() {
        let lines = harness_status_lines(&[], &harness(true, 12, 2000));
        assert_eq!(lines, vec!["no harness entries yet".to_string()]);
    }

    // The cap silently drops entries past it from every turn. A status view that showed all four
    // as active would be lying about what the model actually sees.
    #[test]
    fn harness_status_marks_entries_beyond_the_cap_as_not_injected() {
        let entries: Vec<_> = (0..4)
            .map(|i| entry(&format!("id{i}"), "global", "short"))
            .collect();
        let lines = harness_status_lines(&entries, &harness(true, 2, 2000));

        assert!(lines[0].contains("injecting 2 of 4"), "{}", lines[0]);
        let body = lines[1..].join("\n");
        assert_eq!(body.matches("[injected]").count(), 2);
        assert_eq!(
            body.matches("NOT INJECTED — beyond the entry cap").count(),
            2
        );
    }

    // `harness.enabled = false` stops injection entirely while entries stay stored, so no row may
    // claim to be injected.
    #[test]
    fn harness_status_says_nothing_is_injected_when_disabled() {
        let entries = vec![entry("id0", "global", "short")];
        let lines = harness_status_lines(&entries, &harness(false, 12, 2000));

        assert!(lines[0].contains("OFF"), "{}", lines[0]);
        assert!(!lines.iter().any(|l| l.contains("[injected]")));
        assert!(lines[1..].join("\n").contains("harness off"));
    }

    #[test]
    fn harness_status_flags_content_the_injection_would_truncate() {
        let entries = vec![
            entry("id0", "global", &"x".repeat(50)),
            entry("id1", "global", "short"),
        ];
        let lines = harness_status_lines(&entries, &harness(true, 12, 10));
        let body = lines[1..].join("\n");

        assert_eq!(body.matches("injected, content truncated").count(), 1);
        assert_eq!(body.matches("[injected]").count(), 1);
    }

    // Truncation is measured in chars, matching context_pipeline::clamp_chars — a byte-length
    // check would over-report on any entry containing non-ASCII.
    #[test]
    fn harness_status_counts_truncation_in_chars_not_bytes() {
        let entries = vec![entry("id0", "global", "ééééé")];
        let lines = harness_status_lines(&entries, &harness(true, 12, 5));
        assert!(
            lines[1..].join("\n").contains("[injected]"),
            "5 chars under a 5-char cap must not read as truncated"
        );
    }

    #[test]
    fn harness_status_groups_by_scope_in_precedence_order() {
        let entries = vec![
            entry("id0", "session:s1", "short"),
            entry("id1", "project:/w", "short"),
            entry("id2", "global", "short"),
        ];
        let lines = harness_status_lines(&entries, &harness(true, 12, 2000));
        let headers: Vec<_> = lines
            .iter()
            .filter(|l| l.starts_with("  ") && !l.starts_with("    "))
            .collect();
        assert_eq!(headers, vec!["  session:s1", "  project:/w", "  global"]);
    }

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
