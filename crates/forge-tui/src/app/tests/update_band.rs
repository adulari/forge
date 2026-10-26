//! The update card above the composer: offered when idle, never over a running turn.

use super::*;
use crate::update_card::{ServiceNote, UpdateAction, UpdateOffer, UpdatePlan};

fn offer(plan: UpdatePlan) -> UpdateOffer {
    UpdateOffer {
        version: "2.24.0".into(),
        current: "2.23.6".into(),
        notes: vec!["Faster compaction".into(), "Fixes the resume picker".into()],
        url: "https://github.com/Adulari/forge/releases/tag/v2.24.0".into(),
        plan,
    }
}

fn app_with(plan: UpdatePlan) -> App {
    let mut app = App {
        fullscreen: true,
        ..Default::default()
    };
    app.update_card.offer(offer(plan));
    app
}

fn row_of(rows: &[String], needle: &str) -> usize {
    rows.iter()
        .position(|r| r.contains(needle))
        .unwrap_or_else(|| panic!("{needle:?} missing: {rows:#?}"))
}

#[test]
fn an_idle_session_shows_the_card_with_notes_link_and_keys_above_the_composer() {
    let app = app_with(UpdatePlan::InPlace);
    let rows = screen_rows(&app, 80, 24);
    let title = row_of(&rows, "Forge 2.24.0 is available (you have 2.23.6)");
    let composer = row_of(&rows, "╭");
    assert!(rows[title + 1].contains("- Faster compaction"), "{rows:#?}");
    assert!(rows.iter().any(|r| r.contains("releases/tag/v2.24.0")));
    let keys = row_of(&rows, "Update now");
    assert!(rows[keys].contains("Later") && rows[keys].contains("Skip this version"));
    assert!(title < keys && keys < composer, "{rows:#?}");
}

#[test]
fn a_running_turn_is_never_interrupted_by_the_card() {
    let mut app = app_with(UpdatePlan::InPlace);
    app.busy = true;
    let rows = screen_rows(&app, 80, 24);
    assert!(
        !rows.iter().any(|r| r.contains("is available")),
        "{rows:#?}"
    );
    app.busy = false;
    let rows = screen_rows(&app, 80, 24);
    assert!(rows.iter().any(|r| r.contains("is available")));
}

#[test]
fn a_package_manager_install_shows_its_exact_command() {
    let app = app_with(UpdatePlan::Command("brew upgrade forge".into()));
    let rows = screen_rows(&app, 80, 24);
    assert!(rows.iter().any(|r| r.contains("run: brew upgrade forge")));
    let keys = &rows[row_of(&rows, "Run it now")];
    assert!(keys.contains("Copy command"), "{keys}");
}

#[test]
fn a_draft_in_the_composer_turns_the_keys_into_a_hint() {
    let mut app = app_with(UpdatePlan::InPlace);
    app.input = "half a thought".into();
    let rows = screen_rows(&app, 80, 24);
    assert!(rows
        .iter()
        .any(|r| r.contains("finish or clear your message")));
    assert!(!rows.iter().any(|r| r.contains("Update now")));
}

#[test]
fn progress_done_and_failure_each_have_their_own_card() {
    let mut app = app_with(UpdatePlan::InPlace);
    assert_eq!(
        app.update_card.handle_key(KeyKind::Char('u')),
        UpdateAction::StartInPlace
    );
    app.update_card.progress("downloading…");
    let rows = screen_rows(&app, 80, 24);
    assert!(
        rows.iter().any(|r| r.contains("Updating Forge to 2.24.0")),
        "{rows:#?}"
    );
    assert!(rows.iter().any(|r| r.contains("downloading…")));

    app.update_card.finished(Ok("2.24.0".into()));
    app.update_card.set_service(ServiceNote::Running);
    let rows = screen_rows(&app, 80, 24);
    assert!(
        rows.iter().any(|r| r.contains("Updated to 2.24.0")),
        "{rows:#?}"
    );
    let keys = &rows[row_of(&rows, "Restart now")];
    assert!(
        keys.contains("Restart the Forge service") && keys.contains("Later"),
        "{keys}"
    );

    app.update_card.finished(Err("permission denied".into()));
    let rows = screen_rows(&app, 80, 24);
    assert!(rows
        .iter()
        .any(|r| r.contains("Update failed: permission denied")));
    assert!(rows.iter().any(|r| r.contains("Retry")));
}

#[test]
fn the_card_fits_a_short_narrow_terminal_without_dropping_the_keys() {
    let app = app_with(UpdatePlan::InPlace);
    for (w, h) in [(40u16, 12u16), (60, 10)] {
        let rows = screen_rows(&app, w, h);
        assert!(
            rows.iter().any(|r| r.contains("is available")),
            "{w}x{h}: {rows:#?}"
        );
        assert!(
            rows.iter().any(|r| r.contains("Update now")),
            "{w}x{h}: {rows:#?}"
        );
        assert!(rows.iter().all(|r| r.chars().count() <= w as usize));
    }
}
