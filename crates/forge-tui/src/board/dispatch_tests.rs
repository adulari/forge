//! Tests for plan & dispatch on the board: classification, the form, the approval checklist,
//! revise/cancel, zoom, merge/discard, auto-select after start, and what each screen draws.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::backend::TestBackend;
use ratatui::Terminal;

use super::dispatch::{self, toggle, Role};
use super::form::{FormField, FormHit};
use super::state::{BoardApp, Button, ConfirmKind, DetailTab, Focus, Hit, ToastLevel};
use super::*;

const NOW: i64 = 1_700_000_000;
const CWD: &str = "/home/dev/forge";

// --------------------------------------------------------------------- fixtures

fn row(id: &str, title: &str) -> FleetRow {
    FleetRow {
        id: id.into(),
        title: title.into(),
        cwd: CWD.into(),
        model: "anthropic::claude-opus".into(),
        last_activity: NOW,
        created_at: NOW - 600,
        ..Default::default()
    }
}

fn worker_row(id: &str, index: usize) -> FleetRow {
    FleetRow {
        dispatch_id: Some("disp-1".into()),
        dispatch_role: Some("worker".into()),
        dispatch_index: Some(index),
        worktree: Some(format!("/home/dev/.forge/worktrees/wt-{index}")),
        model: "openai::gpt-5".into(),
        ..row(id, &format!("part {index}"))
    }
}

fn item(
    index: usize,
    title: &str,
    deps: &[usize],
    status: &str,
    session: Option<&str>,
) -> DispatchItemInfo {
    DispatchItemInfo {
        index,
        title: title.into(),
        prompt: format!("Do the {title} part.\nWith a second line of detail."),
        depends_on: deps.to_vec(),
        status: status.into(),
        session_id: session.map(str::to_string),
        ..Default::default()
    }
}

fn dispatch_with(status: &str, items: Vec<DispatchItemInfo>) -> DispatchInfo {
    DispatchInfo {
        id: "disp-1".into(),
        coordinator_session_id: "coord".into(),
        coordinator_title: "Dispatch: split the parser work".into(),
        cwd: CWD.into(),
        prompt: "Split the parser work into parallel parts".into(),
        summary: "Three parts: notes, tasks and a summary that needs the notes.".into(),
        status: status.into(),
        worktree: true,
        max_running: 4,
        max_items: 8,
        items,
        ..Default::default()
    }
}

fn proposed() -> DispatchInfo {
    dispatch_with(
        "proposed",
        vec![
            item(1, "Notes file", &[], "proposed", None),
            item(2, "Tasks", &[], "proposed", None),
            item(3, "Summary", &[1], "proposed", None),
        ],
    )
}

fn running() -> DispatchInfo {
    dispatch_with(
        "running",
        vec![
            item(1, "Notes file", &[], "succeeded", Some("w1")),
            item(2, "Tasks", &[], "running", Some("w2")),
            item(3, "Summary", &[2], "queued", None),
        ],
    )
}

fn board(d: DispatchInfo) -> BoardApp {
    let mut app = BoardApp::new(Some(CWD.into()), NOW);
    let mut w2 = worker_row("w2", 2);
    w2.busy = true;
    let rows = if d.status == "running" || d.status == "done" {
        vec![
            row("coord", "Dispatch: split the parser work"),
            worker_row("w1", 1),
            w2,
        ]
    } else {
        vec![row("coord", "Dispatch: split the parser work")]
    };
    app.apply(BoardEvent::Fleet(rows));
    app.apply(BoardEvent::Dispatches(vec![d]));
    app
}

fn open_tab(app: &mut BoardApp, id: &str) {
    app.select(id);
    app.open_detail();
    app.detail_tab = DetailTab::Dispatch;
    app.focus = Focus::Detail;
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ch(c: char) -> KeyEvent {
    key(KeyCode::Char(c))
}

fn type_text(app: &mut BoardApp, text: &str) {
    for c in text.chars() {
        handle_key(app, ch(c));
    }
}

fn screen(app: &mut BoardApp, w: u16, h: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("terminal");
    terminal.draw(|f| app.draw(f)).expect("draw");
    let buf = terminal.backend().buffer().clone();
    (0..buf.area.height)
        .map(|y| {
            (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn click_hit(app: &mut BoardApp, hit: &Hit) -> Vec<BoardAction> {
    let rect = app
        .hits
        .iter()
        .rev()
        .find(|(_, h)| h == hit)
        .map(|(r, _)| *r)
        .unwrap_or_else(|| panic!("no hit for {hit:?}"));
    handle_mouse(
        app,
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: rect.x,
            row: rect.y,
            modifiers: KeyModifiers::NONE,
        },
    )
}

fn last_toast(app: &BoardApp) -> String {
    app.toasts
        .back()
        .map(|t| t.text.clone())
        .unwrap_or_default()
}

// ============================================================ classification

#[test]
fn a_planning_coordinator_is_working() {
    let app = board(dispatch_with("planning", Vec::new()));
    let c = app.card("coord").unwrap();
    assert_eq!(c.column, Column::Working);
    assert_eq!(c.dispatch.as_ref().unwrap().role, Role::Coordinator);
}

#[test]
fn a_proposed_split_needs_you_even_though_the_coordinator_is_idle() {
    let app = board(proposed());
    let c = app.card("coord").unwrap();
    assert!(!c.busy);
    assert_eq!(c.column, Column::Attention);
    let s = c.signals.first().unwrap();
    assert_eq!(s.level, SignalLevel::Danger);
    assert_eq!(s.text, "split ready · 3 sessions to review");
}

#[test]
fn a_running_dispatch_keeps_its_idle_coordinator_in_working() {
    let app = board(running());
    assert_eq!(app.card("coord").unwrap().column, Column::Working);
}

#[test]
fn a_finished_dispatch_is_ready_and_a_failure_warns() {
    let mut d = running();
    d.status = "done".into();
    d.items[1].status = "failed".into();
    d.items[2].status = "succeeded".into();
    let app = board(d);
    let c = app.card("coord").unwrap();
    assert_eq!(c.column, Column::Ready);
    assert!(c
        .signals
        .iter()
        .any(|s| s.level == SignalLevel::Warn && s.text.contains("did not finish")));
}

#[test]
fn a_cancelled_dispatch_is_ready() {
    let app = board(dispatch_with("cancelled", Vec::new()));
    assert_eq!(app.card("coord").unwrap().column, Column::Ready);
}

#[test]
fn a_coordinator_waiting_on_a_permission_stays_in_needs_you() {
    let mut app = board(running());
    app.apply(BoardEvent::Snapshot(
        "coord".into(),
        LiveSnapshot {
            session_id: "coord".into(),
            permission_prompt: Some("write notes.md?".into()),
            ..Default::default()
        },
    ));
    assert_eq!(app.card("coord").unwrap().column, Column::Attention);
}

#[test]
fn workers_are_recognised_from_their_row_or_from_the_item() {
    let app = board(running());
    let w = app.card("w2").unwrap().dispatch.clone().unwrap();
    assert_eq!(w.role, Role::Worker(2));
    assert_eq!(w.total, 3);

    // An older daemon's row carries no dispatch fields; the item's session id still says so.
    let mut app = BoardApp::new(None, NOW);
    app.apply(BoardEvent::Fleet(vec![
        row("coord", "c"),
        row("w1", "bare"),
    ]));
    app.apply(BoardEvent::Dispatches(vec![running()]));
    assert_eq!(
        app.card("w1").unwrap().dispatch.as_ref().unwrap().role,
        Role::Worker(1)
    );
}

#[test]
fn a_dispatchs_colour_is_stable_and_never_an_alarm_colour() {
    assert_eq!(group_color("disp-1"), group_color("disp-1"));
    let mut seen = std::collections::HashSet::new();
    for i in 0..300 {
        let c = group_color(&format!("dispatch-{i}"));
        assert_ne!(c, crate::surface::ERRRED);
        assert_ne!(c, crate::surface::WARNYEL);
        seen.insert(format!("{c:?}"));
    }
    assert!(seen.len() >= 4, "ids should spread over the palette");
}

#[test]
fn an_old_daemon_without_dispatches_changes_nothing() {
    let mut app = BoardApp::new(None, NOW);
    app.apply(BoardEvent::Fleet(vec![row("s1", "plain")]));
    app.apply(BoardEvent::Dispatches(Vec::new()));
    let c = app.card("s1").unwrap();
    assert!(c.dispatch.is_none());
    assert_eq!(c.column, Column::Ready);
}

// ============================================================ the form

#[test]
fn d_opens_the_form_in_the_selected_cards_project() {
    let mut app = BoardApp::new(Some("/elsewhere".into()), NOW);
    app.apply(BoardEvent::Fleet(vec![row("s1", "x")]));
    app.select("s1");
    handle_key(&mut app, ch('D'));
    assert_eq!(app.focus(), Focus::Form);
    let form = app.dispatch.form.as_ref().unwrap();
    assert_eq!(form.cwd, CWD);
    assert_eq!(form.field, FormField::Prompt);
    assert!(form.worktree);
    assert_eq!(form.mode_key(), "accept-edits");
    assert_eq!((form.max_running, form.max_items), (4, 8));

    let mut empty = BoardApp::new(Some("/board/cwd".into()), NOW);
    handle_key(&mut empty, ch('D'));
    assert_eq!(empty.dispatch.form.as_ref().unwrap().cwd, "/board/cwd");
}

#[test]
fn the_prompt_box_takes_text_newlines_and_multiline_pastes() {
    let mut app = BoardApp::new(Some(CWD.into()), NOW);
    handle_key(&mut app, ch('D'));
    type_text(&mut app, "split it");
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL),
    );
    handle_paste(&mut app, "line two\r\nline three");
    assert_eq!(
        app.dispatch.form.as_ref().unwrap().text.text,
        "split it\nline two\nline three"
    );
    // ↑ moves within the box instead of leaving it.
    handle_key(&mut app, key(KeyCode::Up));
    let f = app.dispatch.form.as_ref().unwrap();
    assert_eq!(f.field, FormField::Prompt);
    assert_eq!(f.cursor_line_col().0, 1);
}

#[test]
fn tab_and_arrows_move_between_fields_and_change_them() {
    let mut app = BoardApp::new(Some(CWD.into()), NOW);
    handle_key(&mut app, ch('D'));
    handle_key(&mut app, key(KeyCode::Down));
    assert_eq!(
        app.dispatch.form.as_ref().unwrap().field,
        FormField::Worktree
    );
    handle_key(&mut app, ch(' '));
    assert!(!app.dispatch.form.as_ref().unwrap().worktree);
    handle_key(&mut app, key(KeyCode::Tab));
    handle_key(&mut app, key(KeyCode::Right));
    assert_eq!(app.dispatch.form.as_ref().unwrap().mode_key(), "bypass");
    handle_key(&mut app, key(KeyCode::Left));
    handle_key(&mut app, key(KeyCode::Left));
    assert_eq!(app.dispatch.form.as_ref().unwrap().mode_key(), "default");
    handle_key(&mut app, key(KeyCode::Tab));
    for _ in 0..20 {
        handle_key(&mut app, key(KeyCode::Right));
    }
    assert_eq!(app.dispatch.form.as_ref().unwrap().max_running, 8);
    handle_key(&mut app, key(KeyCode::Down));
    for _ in 0..20 {
        handle_key(&mut app, key(KeyCode::Left));
    }
    assert_eq!(app.dispatch.form.as_ref().unwrap().max_items, 1);
    handle_key(&mut app, key(KeyCode::BackTab));
    assert_eq!(
        app.dispatch.form.as_ref().unwrap().field,
        FormField::Running
    );
    handle_key(&mut app, key(KeyCode::Up));
    assert_eq!(app.dispatch.form.as_ref().unwrap().field, FormField::Mode);
}

#[test]
fn an_empty_prompt_is_refused_without_losing_the_form() {
    let mut app = BoardApp::new(Some(CWD.into()), NOW);
    handle_key(&mut app, ch('D'));
    handle_key(&mut app, key(KeyCode::Tab));
    type_text(&mut app, "   ");
    let out = handle_key(&mut app, key(KeyCode::Enter));
    assert!(out.is_empty());
    assert_eq!(app.focus(), Focus::Form);
    assert!(app.form_shaking());
    assert_eq!(app.dispatch.form.as_ref().unwrap().field, FormField::Prompt);
    assert_eq!(last_toast(&app), "describe the work first");
    for _ in 0..super::form::SHAKE_TICKS {
        app.apply(BoardEvent::Tick);
    }
    assert!(!app.form_shaking());
}

#[test]
fn enter_starts_the_dispatch_from_any_field() {
    let mut app = BoardApp::new(Some(CWD.into()), NOW);
    handle_key(&mut app, ch('D'));
    type_text(&mut app, "split the parser work");
    handle_key(&mut app, key(KeyCode::Tab));
    handle_key(&mut app, ch(' '));
    handle_key(&mut app, key(KeyCode::Tab));
    handle_key(&mut app, key(KeyCode::Tab));
    handle_key(&mut app, key(KeyCode::Left));
    let out = handle_key(&mut app, key(KeyCode::Enter));
    assert_eq!(
        out,
        vec![BoardAction::StartDispatch {
            cwd: CWD.into(),
            prompt: "split the parser work".into(),
            worktree: false,
            mode: "accept-edits".into(),
            max_running: 3,
            max_items: 8,
        }]
    );
    assert!(app.dispatch.form.is_none());
    assert_eq!(app.focus(), Focus::Board);
}

#[test]
fn esc_cancels_the_form() {
    let mut app = BoardApp::new(Some(CWD.into()), NOW);
    handle_key(&mut app, ch('D'));
    type_text(&mut app, "x");
    assert!(handle_key(&mut app, key(KeyCode::Esc)).is_empty());
    assert!(app.dispatch.form.is_none());
    assert_eq!(app.focus(), Focus::Board);
}

#[test]
fn the_form_answers_the_mouse_and_ignores_stray_clicks() {
    let mut app = BoardApp::new(Some(CWD.into()), NOW);
    app.apply(BoardEvent::Fleet(vec![row("s1", "x")]));
    screen(&mut app, 120, 40);
    click_hit(&mut app, &Hit::DispatchChip);
    assert_eq!(app.focus(), Focus::Form);
    screen(&mut app, 120, 40);
    click_hit(&mut app, &Hit::Form(FormHit::Step(FormField::Running, 1)));
    let f = app.dispatch.form.as_ref().unwrap();
    assert_eq!((f.field, f.max_running), (FormField::Running, 5));
    click_hit(&mut app, &Hit::Form(FormHit::Worktree(false)));
    assert!(!app.dispatch.form.as_ref().unwrap().worktree);
    // A click on the board behind the form keeps the half-written form.
    handle_mouse(
        &mut app,
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 0,
            row: 39,
            modifiers: KeyModifiers::NONE,
        },
    );
    assert!(app.dispatch.form.is_some());
    type_text(&mut app, "go");
    screen(&mut app, 120, 40);
    let out = click_hit(&mut app, &Hit::Form(FormHit::Start));
    assert!(matches!(
        out.as_slice(),
        [BoardAction::StartDispatch { .. }]
    ));
}

// ============================================================ auto-select after start

#[test]
fn the_started_coordinator_is_selected_and_opened_the_moment_it_appears() {
    let mut app = BoardApp::new(Some(CWD.into()), NOW);
    app.apply(BoardEvent::Fleet(vec![row("other", "x")]));
    app.set_project_filter(Some("unrelated".into()));
    handle_key(&mut app, ch('D'));
    type_text(&mut app, "split");
    handle_key(&mut app, key(KeyCode::Enter));
    app.apply(BoardEvent::DispatchStarted {
        dispatch_id: "disp-1".into(),
        coordinator_session_id: "coord".into(),
    });
    assert_eq!(app.project_filter(), None, "a filter must not hide it");
    assert_ne!(app.selected.as_deref(), Some("coord"));
    app.apply(BoardEvent::Fleet(vec![
        row("other", "x"),
        row("coord", "Dispatch: split"),
    ]));
    assert_eq!(app.selected.as_deref(), Some("coord"));
    assert!(app.detail_open);
    assert_eq!(app.focus(), Focus::Detail);
    app.apply(BoardEvent::Dispatches(vec![dispatch_with(
        "planning",
        Vec::new(),
    )]));
    assert_eq!(app.effective_tab(), DetailTab::Dispatch);
    // Only once: the user can move away afterwards.
    app.select("other");
    app.apply(BoardEvent::Fleet(vec![
        row("other", "x"),
        row("coord", "Dispatch: split"),
    ]));
    assert_eq!(app.selected.as_deref(), Some("other"));
}

// ============================================================ the approval checklist

#[test]
fn toggling_drags_dependencies_and_dependants_along() {
    let items = proposed().items;
    let mut sel: std::collections::BTreeSet<usize> = [1, 2, 3].into();
    assert_eq!(toggle(&items, &mut sel, 1), (false, vec![3]));
    assert_eq!(sel, [2].into());
    assert_eq!(toggle(&items, &mut sel, 3), (true, vec![1]));
    assert_eq!(sel, [1, 2, 3].into());
    assert_eq!(toggle(&items, &mut sel, 2), (false, vec![]));
}

#[test]
fn space_unticks_an_item_and_its_dependants_with_a_toast() {
    let mut app = board(proposed());
    open_tab(&mut app, "coord");
    handle_key(&mut app, ch(' '));
    assert_eq!(app.dispatch_selection("disp-1").unwrap(), &[2].into());
    assert_eq!(last_toast(&app), "also unticked 3 — it needs 1");
    handle_key(&mut app, key(KeyCode::Down));
    handle_key(&mut app, key(KeyCode::Down));
    handle_key(&mut app, ch(' '));
    assert_eq!(app.dispatch_selection("disp-1").unwrap(), &[1, 2, 3].into());
    assert_eq!(last_toast(&app), "also ticked 1 — 3 needs it");
}

#[test]
fn y_approves_everything_as_none_and_a_subset_as_a_list() {
    let mut app = board(proposed());
    open_tab(&mut app, "coord");
    assert_eq!(
        handle_key(&mut app, ch('y')),
        vec![BoardAction::ApproveDispatch {
            id: "disp-1".into(),
            selected: None
        }]
    );
    handle_key(&mut app, key(KeyCode::Down));
    handle_key(&mut app, ch(' '));
    assert_eq!(
        handle_key(&mut app, ch('y')),
        vec![BoardAction::ApproveDispatch {
            id: "disp-1".into(),
            selected: Some(vec![1, 3])
        }]
    );
}

#[test]
fn nothing_ticked_means_nothing_starts() {
    let mut app = board(proposed());
    open_tab(&mut app, "coord");
    handle_key(&mut app, ch(' '));
    handle_key(&mut app, key(KeyCode::Down));
    handle_key(&mut app, ch(' '));
    assert!(handle_key(&mut app, ch('y')).is_empty());
    assert_eq!(last_toast(&app), "tick at least one session first");
}

#[test]
fn a_pending_permission_still_owns_y() {
    let mut app = board(proposed());
    app.apply(BoardEvent::Snapshot(
        "coord".into(),
        LiveSnapshot {
            session_id: "coord".into(),
            permission_prompt: Some("read the repo?".into()),
            prompt_seq: 4,
            ..Default::default()
        },
    ));
    open_tab(&mut app, "coord");
    let out = handle_key(&mut app, ch('y'));
    assert!(matches!(out.as_slice(), [BoardAction::Input(id, _)] if id == "coord"));
}

#[test]
fn the_checklist_survives_a_refresh_but_resets_for_a_revised_split() {
    let mut app = board(proposed());
    open_tab(&mut app, "coord");
    handle_key(&mut app, key(KeyCode::Down));
    handle_key(&mut app, ch(' '));
    app.apply(BoardEvent::Dispatches(vec![proposed()]));
    assert_eq!(app.dispatch_selection("disp-1").unwrap(), &[1, 3].into());
    let mut revised = proposed();
    revised.items[1].title = "Tasks, better".into();
    app.apply(BoardEvent::Dispatches(vec![revised]));
    assert_eq!(app.dispatch_selection("disp-1").unwrap(), &[1, 2, 3].into());
}

#[test]
fn enter_unfolds_a_proposed_items_prompt() {
    let mut app = board(proposed());
    open_tab(&mut app, "coord");
    handle_key(&mut app, key(KeyCode::Enter));
    assert_eq!(app.dispatch.expanded, Some(1));
    let s = screen(&mut app, 120, 40);
    assert!(s.contains("With a second line of detail."), "{s}");
    handle_key(&mut app, key(KeyCode::Enter));
    assert_eq!(app.dispatch.expanded, None);
}

#[test]
fn e_revises_a_proposed_split() {
    let mut app = board(proposed());
    open_tab(&mut app, "coord");
    handle_key(&mut app, ch('e'));
    assert_eq!(app.focus(), Focus::Composer);
    type_text(&mut app, "merge 2 into 1");
    assert_eq!(
        handle_key(&mut app, key(KeyCode::Enter)),
        vec![BoardAction::ReviseDispatch {
            id: "disp-1".into(),
            feedback: "merge 2 into 1".into()
        }]
    );
}

#[test]
fn n_asks_before_cancelling_a_proposed_split() {
    let mut app = board(proposed());
    open_tab(&mut app, "coord");
    assert!(handle_key(&mut app, ch('n')).is_empty());
    assert_eq!(
        app.confirm.as_ref().unwrap().kind,
        ConfirmKind::CancelDispatch("disp-1".into())
    );
    assert_eq!(
        handle_key(&mut app, key(KeyCode::Enter)),
        vec![BoardAction::CancelDispatch("disp-1".into())]
    );
}

#[test]
fn the_buttons_and_boxes_are_clickable() {
    let mut app = board(proposed());
    open_tab(&mut app, "coord");
    screen(&mut app, 120, 40);
    click_hit(&mut app, &Hit::DispatchToggle(1));
    assert_eq!(app.dispatch_selection("disp-1").unwrap(), &[1, 3].into());
    screen(&mut app, 120, 40);
    let out = click_hit(&mut app, &Hit::Button(Button::Approve));
    assert!(matches!(
        out.as_slice(),
        [BoardAction::ApproveDispatch {
            selected: Some(_),
            ..
        }]
    ));
    screen(&mut app, 120, 40);
    click_hit(&mut app, &Hit::DispatchRow(2));
    assert_eq!(app.dispatch.cursor, 2);
}

// ============================================================ running

#[test]
fn enter_on_a_running_row_opens_that_workers_pane() {
    let mut app = board(running());
    open_tab(&mut app, "coord");
    handle_key(&mut app, key(KeyCode::Down));
    let out = handle_key(&mut app, key(KeyCode::Enter));
    assert_eq!(app.selected.as_deref(), Some("w2"));
    assert_eq!(app.detail_tab, DetailTab::Tail);
    assert_eq!(out, vec![BoardAction::WantDetail("w2".into())]);
}

#[test]
fn a_workers_dispatch_tab_starts_on_its_own_row() {
    let mut app = board(running());
    open_tab(&mut app, "w2");
    assert_eq!(app.dispatch.cursor, 1);
}

#[test]
fn merge_all_finished_asks_first_and_is_refused_when_there_is_nothing() {
    let mut app = board(running());
    open_tab(&mut app, "coord");
    handle_key(&mut app, ch('A'));
    assert_eq!(
        app.confirm.as_ref().unwrap().kind,
        ConfirmKind::MergeFinished("disp-1".into())
    );
    assert!(app.confirm.as_ref().unwrap().body.contains("item 1"));
    assert_eq!(
        handle_key(&mut app, key(KeyCode::Enter)),
        vec![BoardAction::MergeFinished("disp-1".into())]
    );

    let mut shared = running();
    shared.worktree = false;
    let mut app = board(shared);
    open_tab(&mut app, "coord");
    handle_key(&mut app, ch('A'));
    assert!(app.confirm.is_none());

    let mut none = running();
    none.items[0].status = "merged".into();
    let mut app = board(none);
    open_tab(&mut app, "coord");
    handle_key(&mut app, ch('A'));
    assert!(app.confirm.is_none());
    assert_eq!(last_toast(&app), "no finished session to merge yet");
}

#[test]
fn n_cancels_the_rest_of_a_running_dispatch_after_asking() {
    let mut app = board(running());
    open_tab(&mut app, "coord");
    handle_key(&mut app, ch('n'));
    let c = app.confirm.as_ref().unwrap();
    assert_eq!(c.kind, ConfirmKind::CancelDispatch("disp-1".into()));
    assert!(
        c.body.contains("1 running session keep going"),
        "{}",
        c.body
    );
}

// ============================================================ zoom

#[test]
fn z_zooms_to_the_dispatch_and_esc_clears_it_before_the_filters() {
    let mut app = board(running());
    app.apply(BoardEvent::Fleet(vec![
        row("coord", "Dispatch: split"),
        worker_row("w1", 1),
        worker_row("w2", 2),
        row("stranger", "unrelated"),
    ]));
    app.select("w1");
    handle_key(&mut app, ch('z'));
    let ids: Vec<&str> = app.cards.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids.len(), 3, "{ids:?}");
    assert!(!ids.contains(&"stranger"));
    app.query = "part".into();
    handle_key(&mut app, key(KeyCode::Esc));
    assert!(app.dispatch.zoom.is_none());
    assert_eq!(app.query, "part");

    app.select("stranger");
    app.query.clear();
    app.rebuild();
    app.select("stranger");
    handle_key(&mut app, ch('z'));
    assert!(app.dispatch.zoom.is_none());
    assert_eq!(last_toast(&app), "this card is not part of a dispatch");
}

#[test]
fn the_zoom_chip_names_the_dispatch_and_clears_on_click() {
    let mut app = board(running());
    app.select("w1");
    handle_key(&mut app, ch('z'));
    let s = screen(&mut app, 160, 40);
    assert!(s.contains("◆ split the parser work ✕"), "{s}");
    click_hit(&mut app, &Hit::ClearZoom);
    assert!(app.dispatch.zoom.is_none());
}

// ============================================================ merge / discard

#[test]
fn w_asks_before_merging_a_worktree_session() {
    let mut app = board(running());
    app.select("w1");
    handle_key(&mut app, ch('w'));
    let c = app.confirm.clone().unwrap();
    assert_eq!(c.kind, ConfirmKind::Merge("w1".into()));
    assert!(c.body.contains("stops"));
    assert!(c.body.contains("worktree and branch are removed"));
    assert!(c.body.contains("Conflicts leave everything as it was"));
    assert!(!c.kind.is_danger());
    assert_eq!(
        handle_key(&mut app, key(KeyCode::Enter)),
        vec![BoardAction::Merge("w1".into())]
    );
}

#[test]
fn x_asks_before_discarding_in_the_danger_tone() {
    let mut app = board(running());
    app.select("w2");
    handle_key(&mut app, ch('X'));
    let c = app.confirm.clone().unwrap();
    assert_eq!(c.kind, ConfirmKind::Discard("w2".into()));
    assert!(c.kind.is_danger());
    assert!(c.body.contains("cannot be undone"));
    assert!(handle_key(&mut app, key(KeyCode::Esc)).is_empty());
}

#[test]
fn merge_and_discard_refuse_sessions_they_cannot_act_on() {
    let mut app = BoardApp::new(None, NOW);
    app.apply(BoardEvent::Fleet(vec![
        row("plain", "no worktree"),
        FleetRow {
            terminal: true,
            worktree: Some("/wt".into()),
            ..row("term", "terminal")
        },
    ]));
    app.apply(BoardEvent::Past(vec![PastRow {
        id: "old".into(),
        worktree: Some("/wt".into()),
        ..Default::default()
    }]));
    for (id, k) in [("plain", 'w'), ("term", 'X'), ("old", 'w')] {
        app.select(id);
        handle_key(&mut app, ch(k));
        assert!(app.confirm.is_none(), "{id} must be refused");
        assert_eq!(app.toasts.back().unwrap().level, ToastLevel::Info);
    }
}

// ============================================================ keys, help, keybar

#[test]
fn the_help_lists_every_new_key() {
    for k in ["D", "z", "w", "X", "Space", "A"] {
        assert!(HELP.iter().any(|(key, _)| *key == k), "missing {k}");
    }
}

#[test]
fn brackets_walk_the_tabs_including_dispatch_only_on_dispatch_cards() {
    let mut app = board(running());
    open_tab(&mut app, "coord");
    handle_key(&mut app, ch('['));
    assert_eq!(app.detail_tab, DetailTab::Tools);
    handle_key(&mut app, ch(']'));
    assert_eq!(app.detail_tab, DetailTab::Dispatch);
    app.detail_tab = DetailTab::Dispatch;
    app.apply(BoardEvent::Fleet(vec![row("plain", "p")]));
    app.select("plain");
    assert_eq!(app.effective_tab(), DetailTab::Overview);
    handle_key(&mut app, ch(']'));
    assert_eq!(app.detail_tab, DetailTab::Tail);
}

// ============================================================ render

#[test]
fn a_coordinator_card_says_what_its_dispatch_is_doing() {
    let cases = [
        (dispatch_with("planning", Vec::new()), "splitting the work…"),
        (proposed(), "split ready · 3 sessions to review"),
        (running(), "1/3 done · 1 running · 1 waiting"),
        (dispatch_with("cancelled", Vec::new()), "cancelled"),
    ];
    for (d, want) in cases {
        let mut app = board(d);
        let s = screen(&mut app, 160, 40);
        assert!(s.contains(want), "want {want}: {s}");
        assert!(
            s.contains("◆ split the parser work") || s.contains("◆ Dispatch"),
            "{s}"
        );
    }
    let mut done = running();
    done.status = "done".into();
    done.items[1].status = "failed".into();
    done.items[2].status = "succeeded".into();
    let mut app = board(done);
    let s = screen(&mut app, 160, 40);
    assert!(s.contains("all 3 finished · 2 ✓ 1 ✗"), "{s}");
}

#[test]
fn a_worker_card_carries_its_dispatch_chip() {
    let mut app = board(running());
    let s = screen(&mut app, 160, 40);
    assert!(s.contains("◆ 2/3 gpt-5"), "{s}");
}

#[test]
fn the_form_draws_every_field_at_a_roomy_and_a_small_size() {
    for (w, h) in [(120, 40), (70, 24)] {
        let mut app = BoardApp::new(Some(CWD.into()), NOW);
        handle_key(&mut app, ch('D'));
        let s = screen(&mut app, w, h);
        for want in [
            "Plan & dispatch",
            "in forge",
            "Describe the work.",
            "Worktrees",
            "● one per session",
            "○ shared directory",
            "Sessions may",
            "‹ edit files, ask before shell ›",
            "Run at once",
            "‹ 4 ›",
            "At most",
            "‹ 8 › sessions",
            "Enter  start",
            "Esc cancel",
        ] {
            assert!(s.contains(want), "{w}x{h} missing {want}:\n{s}");
        }
    }
}

#[test]
fn the_dispatch_tab_draws_planning_proposed_and_running() {
    let mut app = board(dispatch_with("planning", Vec::new()));
    open_tab(&mut app, "coord");
    let s = screen(&mut app, 120, 40);
    assert!(
        s.contains("Reading the project and splitting the work…"),
        "{s}"
    );
    assert!(
        s.contains("Split the parser work into parallel parts"),
        "{s}"
    );

    for (w, h) in [(120, 40), (70, 24)] {
        let mut app = board(proposed());
        open_tab(&mut app, "coord");
        let s = screen(&mut app, w, h);
        for want in [
            "[✓]",
            "Notes file",
            "after 1",
            "3 of 3 selected",
            "start 3 sessions",
            "revise",
        ] {
            assert!(s.contains(want), "{w}x{h} missing {want}:\n{s}");
        }
        assert!(app
            .hits
            .iter()
            .any(|(_, h)| *h == Hit::Button(Button::Approve)));
    }

    for (w, h) in [(120, 40), (70, 24)] {
        let mut app = board(running());
        open_tab(&mut app, "coord");
        let s = screen(&mut app, w, h);
        for want in [
            "1/3 done",
            "waits for 2",
            "merge all finished",
            "cancel remaining",
            "Dispatch 1/3",
        ] {
            assert!(s.contains(want), "{w}x{h} missing {want}:\n{s}");
        }
    }
}

#[test]
fn the_header_chip_and_the_empty_board_offer_dispatch() {
    let mut app = BoardApp::new(Some(CWD.into()), NOW);
    let s = screen(&mut app, 100, 24);
    assert!(s.contains("plan & dispatch"), "{s}");
    assert!(s.contains("◆ dispatch"), "{s}");
    assert!(app.hits.iter().any(|(_, h)| *h == Hit::DispatchChip));
}

#[test]
fn nothing_panics_at_the_smallest_sizes() {
    for (w, h) in [(20, 5), (8, 4), (30, 8), (70, 24)] {
        let mut app = board(proposed());
        open_tab(&mut app, "coord");
        screen(&mut app, w, h);
        handle_key(&mut app, ch('D'));
        type_text(
            &mut app,
            &"a long prompt that wraps around several times ".repeat(8),
        );
        screen(&mut app, w, h);
        handle_key(&mut app, key(KeyCode::Esc));
        handle_key(&mut app, ch('n'));
        screen(&mut app, w, h);
        let mut app = board(running());
        open_tab(&mut app, "w2");
        screen(&mut app, w, h);
    }
}

#[test]
fn prompt_layout_wraps_words_and_keeps_the_cursor_on_screen() {
    use super::form_render::{cursor_row, layout};
    assert_eq!(layout("", 10), vec![(0, 0)]);
    assert_eq!(layout("hello world", 8), vec![(0, 6), (6, 11)]);
    assert_eq!(layout("ab\ncd", 8), vec![(0, 2), (3, 5)]);
    let rows = layout("abcd", 4);
    assert_eq!(rows, vec![(0, 4), (4, 4)]);
    assert_eq!(cursor_row(&rows, 4), 1);
    assert_eq!(cursor_row(&layout("hello world", 8), 6), 1);
    let _ = dispatch::title(&proposed());
}
