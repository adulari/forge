use super::*;

fn step(step: &str, status: StepStatus, detail: &str) -> ShipStepLine {
    ShipStepLine {
        step: step.into(),
        status,
        detail: detail.into(),
    }
}

fn shipped() -> ShipOutcome {
    ShipOutcome {
        ok: true,
        steps: vec![
            step("branch", StepStatus::Done, "shipping `feat`"),
            step("commit", StepStatus::Done, "abc1234 feat: cart"),
            step("push", StepStatus::Done, "pushed `feat`"),
            step("pr", StepStatus::Done, "PR #9 opened"),
            step("watch", StepStatus::Done, "watching PR #9"),
        ],
        refused: None,
        pr_number: Some(9),
        pr_url: Some("https://github.com/a/b/pull/9".into()),
        ..ShipOutcome::default()
    }
}

#[test]
fn the_daemons_answer_is_read_into_an_outcome() {
    let value = serde_json::json!({
        "ok": false,
        "steps": [
            {"step": "branch", "status": "done", "detail": "shipping `feat`"},
            {"step": "pr", "status": "refused", "detail": "no write access"}
        ],
        "refused": {"step": "pr", "reason": "no write access"},
        "pr": null
    });
    let outcome = ShipOutcome::from_json(&value);
    assert!(!outcome.ok);
    assert_eq!(outcome.steps[1].status, StepStatus::Refused);
    assert_eq!(
        outcome.refused,
        Some(("pr".into(), "no write access".into()))
    );
    assert_eq!(outcome.pr_number, None);
    let lines = outcome.summary_lines();
    assert!(lines[1].starts_with("STOPPED pr"));
    assert_eq!(lines[2], "Stopped at open pull request: no write access");
    let ok = ShipOutcome::from_json(&serde_json::json!({
        "ok": true, "steps": [], "refused": null,
        "pr": {"number": 9, "url": "https://github.com/a/b/pull/9"}
    }));
    assert!(ok.ok && ok.refused.is_none());
    assert_eq!(ok.pr_number, Some(9));
}

fn previewed(files: &[(&str, &str)]) -> ShipOutcome {
    ShipOutcome {
        ok: true,
        preview: true,
        plan: Some(ShipPlan {
            source: "session".into(),
            files: files
                .iter()
                .map(|(p, k)| (p.to_string(), k.to_string()))
                .collect(),
            left_out: vec!["other.rs".into()],
        }),
        ..ShipOutcome::default()
    }
}

fn confirmable_menu() -> ShipMenu {
    let mut menu = ShipMenu::default();
    menu.open();
    menu.finish_preview(Ok(previewed(&[("a.rs", "modified"), ("b.rs", "created")])));
    menu
}

#[test]
fn opening_asks_for_a_preview_and_enter_waits_for_it() {
    let mut menu = ShipMenu::default();
    menu.open();
    assert!(menu.previewing && menu.needs_preview);
    assert_eq!(
        menu.key(KeyKind::Enter),
        ShipAction::None,
        "no shipping blind"
    );
    assert!(!menu.running);
    menu.finish_preview(Ok(previewed(&[("a.rs", "modified")])));
    assert!(menu.confirmable());
}

#[test]
fn the_preview_lists_the_exact_files_and_what_is_left_out() {
    let menu = confirmable_menu();
    let lines = menu.plan.unwrap().lines(10);
    assert_eq!(lines[0], "Commits 2 file(s) from this session's files:");
    assert_eq!(lines[1], "  M a.rs");
    assert_eq!(lines[2], "  A b.rs");
    assert_eq!(lines[3], "Left out: other.rs");
    let json = ShipOutcome::from_json(&serde_json::json!({
        "ok": true, "preview": true, "steps": [], "refused": null,
        "plan": {"source": "index", "files": [{"path": "x", "kind": "deleted"}], "left_out": []}
    }));
    assert!(json.preview);
    assert_eq!(
        json.plan.unwrap().files,
        [("x".to_string(), "deleted".to_string())]
    );
}

#[test]
fn a_refused_preview_shows_the_reason_and_enter_asks_again() {
    let mut menu = ShipMenu::default();
    menu.open();
    menu.needs_preview = false;
    menu.finish_preview(Ok(ShipOutcome {
        refused: Some((
            "commit".into(),
            "README.md was changed by two sessions".into(),
        )),
        ..ShipOutcome::default()
    }));
    assert!(!menu.confirmable());
    assert_eq!(menu.key(KeyKind::Enter), ShipAction::None);
    assert!(menu.previewing && menu.needs_preview, "retry re-previews");
}

#[test]
fn enter_starts_once_and_the_menu_ignores_keys_while_running() {
    let mut menu = confirmable_menu();
    assert_eq!(menu.key(KeyKind::Enter), ShipAction::Start);
    assert!(menu.running);
    assert_eq!(menu.key(KeyKind::Enter), ShipAction::None);
    assert_eq!(menu.key(KeyKind::Esc), ShipAction::None);
    assert!(menu.open);
}

#[test]
fn a_refusal_can_be_retried_and_a_success_closes_on_enter() {
    let mut menu = confirmable_menu();
    menu.key(KeyKind::Enter);
    menu.finish(Ok(ShipOutcome {
        ok: false,
        steps: vec![step("branch", StepStatus::Refused, "create a branch first")],
        refused: Some(("branch".into(), "create a branch first".into())),
        ..ShipOutcome::default()
    }));
    assert!(!menu.running && !menu.succeeded());
    assert_eq!(
        menu.key(KeyKind::Enter),
        ShipAction::None,
        "retry previews first"
    );
    assert!(menu.previewing);
    menu.finish_preview(Ok(previewed(&[("a.rs", "modified")])));
    assert_eq!(menu.key(KeyKind::Enter), ShipAction::Start);
    menu.finish(Ok(shipped()));
    assert!(menu.succeeded());
    assert_eq!(menu.key(KeyKind::Enter), ShipAction::Close);
    assert!(!menu.open);
}

#[test]
fn a_transport_error_is_shown_and_retryable() {
    let mut menu = confirmable_menu();
    menu.key(KeyKind::Enter);
    menu.finish(Err("could not reach the daemon".into()));
    assert_eq!(menu.error.as_deref(), Some("could not reach the daemon"));
    assert_eq!(menu.key(KeyKind::Char('r')), ShipAction::None);
    assert!(menu.error.is_none() && menu.previewing);
}

#[test]
fn rows_follow_the_steps_and_block_after_a_refusal() {
    assert!(rows(None, false)
        .iter()
        .all(|r| r.state == RowState::Pending));
    assert_eq!(rows(None, true)[0].state, RowState::Active);
    let done = rows(Some(&shipped()), false);
    assert!(done.iter().all(|r| r.state == RowState::Done));
    assert_eq!(done[3].detail, "watching PR #9");

    let refused = ShipOutcome {
        ok: false,
        steps: vec![
            step("commit", StepStatus::Done, "abc"),
            step("pr", StepStatus::Refused, "no write access"),
        ],
        refused: Some(("pr".into(), "no write access".into())),
        ..ShipOutcome::default()
    };
    let states: Vec<_> = rows(Some(&refused), false)
        .iter()
        .map(|r| r.state)
        .collect();
    assert_eq!(
        states,
        [
            RowState::Done,
            RowState::Blocked,
            RowState::Refused,
            RowState::Blocked
        ]
    );
}

#[test]
fn headlines_name_the_pr_or_the_stopping_step() {
    assert_eq!(
        headline(Some(&shipped()), false),
        "PR #9 is open and being watched. Forge merges it when checks pass."
    );
    let branch = ShipOutcome {
        refused: Some(("branch".into(), "HEAD is detached".into())),
        ..ShipOutcome::default()
    };
    assert_eq!(
        headline(Some(&branch), false),
        "Stopped before committing: HEAD is detached"
    );
    let pr = ShipOutcome {
        refused: Some(("pr".into(), "no write access".into())),
        ..ShipOutcome::default()
    };
    assert_eq!(
        headline(Some(&pr), false),
        "Stopped at open pull request: no write access"
    );
    assert_eq!(headline(None, true), "Shipping…");
}
