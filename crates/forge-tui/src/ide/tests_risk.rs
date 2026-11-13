//! Risk-sorted review: the queue groups look-here before routine, `A` accepts the routine hunks,
//! and `u` takes the accept back.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::Terminal;

use super::hunk::{routine_stage_steps, HunkAction};
use super::model::{risk_totals, Risk};
use super::wire::*;
use super::*;

fn file(path: &str, risk: &str, reason: &str, look: usize, routine: usize) -> WireChangedFile {
    WireChangedFile {
        path: path.into(),
        kind: "modified".into(),
        adds: 2,
        dels: 1,
        content_hash: format!("h-{path}"),
        risk: risk.into(),
        risk_reason: reason.into(),
        look_hunks: look,
        routine_hunks: routine,
    }
}

fn app_with(files: Vec<WireChangedFile>) -> IdeApp {
    let mut app = IdeApp::new("/repo", 1_000_000);
    app.apply(IdeEvent::Reviewed(Default::default()));
    app.apply(IdeEvent::Changes(Ok(ProjectChanges {
        cwd: "/repo".into(),
        sessions: vec![WireSessionChanges {
            session_id: "s1".into(),
            title: "agent".into(),
            branch: "main".into(),
            files,
            ..Default::default()
        }],
    })));
    app
}

fn mixed() -> IdeApp {
    app_with(vec![
        file("Cargo.lock", "routine", "lockfile churn", 0, 2),
        file("src/a_test.rs", "look", "deletes a test", 1, 0),
        file("src/mod.rs", "look", "removes error handling", 1, 3),
        file("src/imports.rs", "routine", "import reorder", 0, 1),
    ])
}

fn press(app: &mut IdeApp, c: char) -> Vec<IdeAction> {
    handle_key(app, KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
}

fn screen(app: &mut IdeApp, w: u16, h: u16) -> String {
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(|f| draw(app, f)).unwrap();
    let buf = term.backend().buffer().clone();
    (0..h)
        .map(|y| {
            (0..w)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn files_that_need_a_look_sort_ahead_of_routine_ones() {
    let app = mixed();
    let order: Vec<&str> = app.rows.iter().map(|r| r.path.as_str()).collect();
    assert_eq!(
        order,
        [
            "src/a_test.rs",
            "src/mod.rs",
            "Cargo.lock",
            "src/imports.rs"
        ]
    );
    assert_eq!(app.rows[0].file.risk, Some(Risk::Look));
    assert_eq!(app.rows[2].file.risk_reason, "lockfile churn");
}

#[test]
fn totals_count_hunks_over_unreviewed_files() {
    let app = mixed();
    let totals = risk_totals(&app.rows);
    assert_eq!((totals.look_hunks, totals.routine_hunks), (2, 6));
    assert_eq!((totals.look_files, totals.routine_files), (2, 2));
    assert_eq!(totals.line(), "2 look here · 6 routine");
    assert!(!risk_totals(&app_with(vec![file("a", "", "", 0, 0)]).rows).ranked);
}

#[test]
fn the_rail_leads_with_the_summary_and_labels_the_groups() {
    let mut app = mixed();
    let s = screen(&mut app, 200, 40);
    assert!(s.contains("2 look here · 6 routine"), "{s}");
    assert!(
        s.contains("A accept routine"),
        "the hints bar names the key\n{s}"
    );
    assert!(s.contains("Look here · 2 files"), "{s}");
    assert!(s.contains("Routine · 2 files"), "{s}");
    assert!(s.contains("deletes a test"), "{s}");
    assert!(s.contains("removes error handling"), "{s}");
    let look = s.find("Look here").unwrap();
    let routine = s.find("Routine ·").unwrap();
    assert!(look < routine, "look-here comes first\n{s}");
}

#[test]
fn a_daemon_without_verdicts_shows_no_risk_ui() {
    let mut app = app_with(vec![file("a.rs", "", "", 0, 0)]);
    let s = screen(&mut app, 200, 40);
    assert!(!s.contains("look here"), "{s}");
    assert!(!s.contains("Routine"), "{s}");
}

#[test]
fn accept_routine_asks_the_host_to_stage_files_with_routine_hunks() {
    let mut app = mixed();
    let actions = press(&mut app, 'A');
    let targets = actions
        .iter()
        .find_map(|a| match a {
            IdeAction::AcceptRoutine(t) => Some(t.clone()),
            _ => None,
        })
        .expect("an accept action");
    let mut paths: Vec<&str> = targets.iter().map(|t| t.path.as_str()).collect();
    paths.sort_unstable();
    assert_eq!(paths, ["Cargo.lock", "src/imports.rs", "src/mod.rs"]);
}

#[test]
fn accept_routine_with_nothing_routine_says_so() {
    let mut app = app_with(vec![file("a.rs", "look", "adds code", 1, 0)]);
    assert!(press(&mut app, 'A').is_empty());
    assert!(app.toast.is_some());
}

#[test]
fn the_outcome_toasts_and_u_unstages_the_same_hunks() {
    let mut app = mixed();
    let step = hunk::UndoStep {
        hunk_header: "@@ -1 +1 @@".into(),
        patch: "p".into(),
    };
    let record = |path: &str| hunk::UndoRecord {
        action: HunkAction::Unstage,
        ..hunk::UndoRecord::revert("s1".into(), path.into(), "x".into(), vec![step.clone()])
    };
    let mut first = record("Cargo.lock");
    first.with = vec![record("src/imports.rs")];
    app.apply(IdeEvent::Routine(RoutineOutcome {
        staged: 2,
        failed: 0,
        undo: Some(first),
    }));
    let text = app
        .toast
        .as_ref()
        .map(|t| t.text.clone())
        .unwrap_or_default();
    assert!(text.contains("staged 2 routine hunks"), "{text}");
    let actions = press(&mut app, 'u');
    let batches: Vec<&Vec<hunk::UndoRecord>> = actions
        .iter()
        .filter_map(|a| match a {
            IdeAction::UnstageHunks(records) => Some(records),
            _ => None,
        })
        .collect();
    assert_eq!(
        batches.len(),
        1,
        "one sequential batch, not a request per hunk"
    );
    let mut paths: Vec<&str> = batches[0].iter().map(|r| r.path.as_str()).collect();
    paths.sort_unstable();
    assert_eq!(paths, ["Cargo.lock", "src/imports.rs"]);
    assert!(batches[0].iter().all(|r| r.action == HunkAction::Unstage));
}

#[test]
fn routine_stage_steps_take_only_routine_hunks_last_first() {
    let hunk = |header: &str, risk: &str, lines: &[&str]| WireHunk {
        header: header.into(),
        lines: lines.iter().map(|l| (*l).to_string()).collect(),
        risk: risk.into(),
        reason: "r".into(),
    };
    let mut file = WireDiffFile {
        path: "src/a.rs".into(),
        kind: "modified".into(),
        hunks: vec![
            hunk("@@ -1,2 +1,2 @@", "routine", &["-use b;", "+use a;"]),
            hunk("@@ -9,1 +9,1 @@", "look", &["-x", "+y"]),
            hunk("@@ -20,1 +20,2 @@", "routine", &[" a", "+"]),
        ],
        ..Default::default()
    };
    let steps = routine_stage_steps(&file);
    let headers: Vec<&str> = steps.iter().map(|s| s.hunk_header.as_str()).collect();
    assert_eq!(headers, ["@@ -20,1 +20,2 @@", "@@ -1,2 +1,2 @@"]);
    assert!(steps[1]
        .patch
        .starts_with("--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,2 +1,2 @@"));
    file.skipped_lines = 3;
    assert!(routine_stage_steps(&file).is_empty());
}
