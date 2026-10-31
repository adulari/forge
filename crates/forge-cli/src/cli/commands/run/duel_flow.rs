//! The picker half of `/duel`: rows for a finished arena (ranked when a judge ran) and what
//! picking one does. Both key loops (the terminal one and the headless driver) call these so the
//! two cannot drift.

use super::*;

/// `pr:` prefix a remote client puts on a branch id to open a PR from it instead of merging.
pub(crate) const OPEN_PR_PREFIX: &str = "pr:";

/// One picker row per candidate, best first when a judge ranked them. `id` is the candidate's
/// worktree branch; the subtitle is `diffstat · tests · seconds · cost · summary`, and a judged
/// row leads with a `judge #N S/10: critique` field (first so the terminal's one-line detail shows
/// it); the phone and desktop views find it by its prefix.
pub(crate) fn duel_picker_rows(report: &forge_core::duel::DuelReport) -> Vec<forge_tui::PickerRow> {
    report
        .display_order()
        .into_iter()
        .map(|index| {
            let c = &report.candidates[index];
            let badge = if c.ok { "✓" } else { "✗" };
            let tests = match c.tests_passed {
                Some(true) => "tests ✓",
                Some(false) => "tests ✗",
                None => "tests –",
            };
            let judgement = report.verdict.as_ref().and_then(|v| v.judgement_for(index));
            let crown = if judgement.is_some_and(|j| j.rank == 1) {
                "★ "
            } else {
                ""
            };
            let stats = format!(
                "{} files +{} -{} · {tests} · {:.1}s · ${:.4} · {}",
                c.files_changed,
                c.added,
                c.removed,
                c.duration_ms as f64 / 1000.0,
                c.cost_usd,
                forge_core::duel_judge::clean_line(&c.summary),
            );
            let subtitle = match judgement {
                Some(j) => format!(
                    "judge #{} {:.1}/10: {} · {stats}",
                    j.rank, j.score, j.critique
                ),
                None => stats,
            };
            forge_tui::PickerRow {
                id: c.branch.clone(),
                title: format!("{crown}{badge} {}", c.model),
                subtitle,
            }
        })
        .collect()
}

/// The picker heading; a judge's verdict rides on it after ` · judge `.
pub(crate) fn duel_picker_heading(report: &forge_core::duel::DuelReport) -> String {
    let mut heading = format!(
        "⚔ duel — pick the winner ({} candidates)",
        report.candidates.len()
    );
    if let Some(v) = &report.verdict {
        heading.push_str(&format!(" · judge {}: {}", v.judge_model, v.rationale));
    } else if let Some(e) = &report.judge_error {
        heading.push_str(&format!(
            " · judge failed: {}",
            forge_core::duel_judge::clean_line(e)
        ));
    }
    heading
}

/// Open the winner picker for a finished duel and keep the report and worktree guards in `state`.
pub(crate) fn open_duel_picker(
    app: &mut forge_tui::App,
    state: &mut PendingDuel,
    result: (
        forge_core::duel::DuelReport,
        Vec<forge_core::worktree::WorktreeGuard>,
    ),
) {
    let (report, guards) = result;
    if report.candidates.is_empty() {
        app.note("⚔ duel produced no usable candidates");
        return;
    }
    let verdict_note = report.verdict.as_ref().map(|v| {
        format!(
            "⚖ judge {} ranked {} candidates (${:.4}): {}",
            v.judge_model,
            v.ranking.len(),
            v.cost_usd,
            v.rationale
        )
    });
    app.picker.open_with(
        forge_tui::PickerKind::Duel,
        &duel_picker_heading(&report),
        duel_picker_rows(&report),
    );
    if let Some(note) = verdict_note {
        app.note(&note);
    }
    *state = Some((report, guards));
}

/// Act on a picked candidate: merge its branch back, or (`open_pr`) push it and open a pull
/// request. Every candidate's outcome is recorded for routing, then the worktrees are dropped.
/// A failed PR puts the picker back so the person can merge instead or try again.
pub(crate) async fn resolve_duel_pick(
    app: &mut forge_tui::App,
    state: &mut PendingDuel,
    fallback_root: &std::path::Path,
    branch: &str,
    open_pr: bool,
) {
    let Some((report, guards)) = state.take() else {
        return;
    };
    let Some(index) = report.candidates.iter().position(|c| c.branch == branch) else {
        app.note("⚠ that candidate is no longer part of this duel");
        *state = Some((report, guards));
        return;
    };
    let repo_root = forge_core::duel::repo_root_of(&guards, fallback_root);
    let winner_model = report.candidates[index].model.clone();
    let outcome = if open_pr {
        match forge_core::duel::open_winner_pr(&report, &guards, index).await {
            Ok(text) => text,
            Err(e) => {
                app.note(&format!("⚠ could not open a PR from {winner_model}: {e}"));
                app.picker.open_with(
                    forge_tui::PickerKind::Duel,
                    &duel_picker_heading(&report),
                    duel_picker_rows(&report),
                );
                *state = Some((report, guards));
                return;
            }
        }
    } else {
        match forge_core::duel::merge_winner(&repo_root, branch) {
            Ok(m) if m.conflicted_files.is_empty() => "merged cleanly".to_string(),
            Ok(m) => format!(
                "merged with conflicts in: {}",
                m.conflicted_files.join(", ")
            ),
            Err(e) => format!("merge failed: {e}"),
        }
    };
    if let Ok(store) = crate::open_store() {
        let repo_key = repo_root.display().to_string();
        for c in &report.candidates {
            let _ =
                store.record_duel_outcome(&repo_key, &c.model, c.branch == branch, &report.task);
        }
    }
    app.note(&format!("⚔ duel winner: {winner_model} — {outcome}"));
    drop(guards);
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_core::duel::{DuelCandidate, DuelReport};
    use forge_core::duel_judge::{JudgeVerdict, Judgement};

    fn cand(model: &str, ok: bool) -> DuelCandidate {
        DuelCandidate {
            model: model.into(),
            child_id: model.into(),
            branch: format!("forge/{model}"),
            files_changed: 2,
            added: 5,
            removed: 1,
            tests_passed: Some(ok),
            duration_ms: 2500,
            cost_usd: 0.0123,
            summary: "did the thing · twice".into(),
            ok,
        }
    }

    fn report(judged: bool) -> DuelReport {
        DuelReport {
            task: "t".into(),
            candidates: vec![cand("a", true), cand("b", false), cand("c", true)],
            verdict: judged.then(|| JudgeVerdict {
                judge_model: "j::m".into(),
                ranking: vec![
                    Judgement {
                        index: 2,
                        rank: 1,
                        score: 9.0,
                        critique: "tidy".into(),
                    },
                    Judgement {
                        index: 0,
                        rank: 2,
                        score: 6.5,
                        critique: "fine".into(),
                    },
                    Judgement {
                        index: 1,
                        rank: 3,
                        score: 2.0,
                        critique: "tests fail".into(),
                    },
                ],
                rationale: "c is tidy".into(),
                cost_usd: 0.001,
            }),
            judge_error: None,
        }
    }

    #[test]
    fn unjudged_rows_keep_the_race_order_and_the_old_shape() {
        let rows = duel_picker_rows(&report(false));
        assert_eq!(
            rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            ["forge/a", "forge/b", "forge/c"]
        );
        assert_eq!(rows[0].title, "✓ a");
        assert_eq!(
            rows[0].subtitle,
            "2 files +5 -1 · tests ✓ · 2.5s · $0.0123 · did the thing - twice"
        );
        assert_eq!(
            duel_picker_heading(&report(false)),
            "⚔ duel — pick the winner (3 candidates)"
        );
    }

    #[test]
    fn judged_rows_are_ranked_and_the_winner_is_crowned() {
        let rows = duel_picker_rows(&report(true));
        assert_eq!(
            rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            ["forge/c", "forge/a", "forge/b"]
        );
        assert_eq!(rows[0].title, "★ ✓ c");
        assert!(rows[0]
            .subtitle
            .starts_with("judge #1 9.0/10: tidy · 2 files +5 -1"));
        assert!(rows[2]
            .subtitle
            .starts_with("judge #3 2.0/10: tests fail · "));
        assert!(duel_picker_heading(&report(true)).ends_with(" · judge j::m: c is tidy"));
    }

    #[test]
    fn a_failed_judge_shows_in_the_heading_but_keeps_the_race_order() {
        let mut r = report(false);
        r.judge_error = Some("judge x failed: 429".into());
        assert!(duel_picker_heading(&r).ends_with(" · judge failed: judge x failed: 429"));
        assert_eq!(duel_picker_rows(&r)[0].id, "forge/a");
    }
}
