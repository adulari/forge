//! Tests for the board's pure half: classification, signals, selection, keys, actions.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use super::model::{context_pct, projects};
use super::*;

const NOW: i64 = 1_000_000;

// --------------------------------------------------------------------- helpers

fn row(id: &str) -> FleetRow {
    FleetRow {
        id: id.to_string(),
        // Fresh by default so a bare `busy: true` override reads as Busy/Working rather than
        // accidentally tripping the "silent for..." staleness signal against `NOW`.
        last_activity: NOW,
        ..Default::default()
    }
}

fn snap(id: &str) -> LiveSnapshot {
    LiveSnapshot {
        session_id: id.to_string(),
        ..Default::default()
    }
}

fn app() -> BoardApp {
    BoardApp::new(None, NOW)
}

/// A board with one live, idle, writable, non-past card `id` selected.
fn app_with_selected(id: &str) -> BoardApp {
    let mut a = app();
    a.apply(BoardEvent::Fleet(vec![row(id)]));
    a.select(id);
    a
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn key_mod(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, mods)
}

fn click_at(app: &mut BoardApp, col: u16, row: u16) -> Vec<BoardAction> {
    handle_mouse(
        app,
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        },
    )
}

fn scroll_at(app: &mut BoardApp, col: u16, row: u16, down: bool) -> Vec<BoardAction> {
    handle_mouse(
        app,
        MouseEvent {
            kind: if down {
                MouseEventKind::ScrollDown
            } else {
                MouseEventKind::ScrollUp
            },
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        },
    )
}

// ============================================================ model: classification

#[test]
fn waiting_row_classifies_as_attention() {
    let r = FleetRow {
        waiting: true,
        ..row("s1")
    };
    let c = live_card(&r, None, NOW);
    assert_eq!(c.health, Health::Waiting);
    assert_eq!(c.column, Column::Attention);
}

#[test]
fn busy_row_classifies_as_working() {
    let r = FleetRow {
        busy: true,
        ..row("s1")
    };
    let c = live_card(&r, None, NOW);
    assert_eq!(c.health, Health::Busy);
    assert_eq!(c.column, Column::Working);
}

#[test]
fn idle_success_classifies_as_ready() {
    let r = FleetRow {
        last_turn_outcome: Some("success".into()),
        ..row("s1")
    };
    let c = live_card(&r, None, NOW);
    assert_eq!(c.health, Health::Idle);
    assert_eq!(c.column, Column::Ready);
}

#[test]
fn idle_failed_classifies_as_attention_with_failed_health() {
    let r = FleetRow {
        last_turn_outcome: Some("failed".into()),
        ..row("s1")
    };
    let c = live_card(&r, None, NOW);
    assert_eq!(c.health, Health::Failed);
    assert_eq!(c.column, Column::Attention);
}

#[test]
fn busy_with_danger_signal_classifies_as_stalled_attention() {
    let r = FleetRow {
        busy: true,
        last_activity: NOW - 700,
        ..row("s1")
    };
    let c = live_card(&r, None, NOW);
    assert!(c.signals.iter().any(|s| s.level == SignalLevel::Danger));
    assert_eq!(c.health, Health::Stalled);
    assert_eq!(c.column, Column::Attention);
}

#[test]
fn past_row_classifies_as_done_finished() {
    let p = PastRow {
        id: "p1".into(),
        ..Default::default()
    };
    let c = past_card(&p);
    assert_eq!(c.health, Health::Finished);
    assert_eq!(c.column, Column::Done);
    assert!(c.past);
}

#[test]
fn snapshot_overrides_fleet_rows_busy_waiting_model_title() {
    let r = FleetRow {
        busy: false,
        title: "row title".into(),
        model: "row-model".into(),
        ..row("s1")
    };
    let s = LiveSnapshot {
        busy: true,
        title: "snap title".into(),
        model: "snap-model".into(),
        permission_prompt: Some("do it?".into()),
        ..snap("s1")
    };
    let c = live_card(&r, Some(&s), NOW);
    assert!(c.busy);
    assert!(c.waiting);
    assert_eq!(c.title, "snap title");
    assert_eq!(c.model, "snap-model");
}

// ============================================================ model: signals

#[test]
fn repeated_opening_requires_three_trailing_same_sentence_rows() {
    let mk = |t: &str| TranscriptRow {
        kind: "assistant".into(),
        text: t.into(),
        ..Default::default()
    };
    let rows = vec![
        mk("Reading the initial file to understand it."),
        mk("Reading the same file once more today."),
        mk("Reading the same file once more today."),
        mk("Reading the same file once more today."),
    ];
    assert_eq!(repeated_opening(&rows), Some(3));
}

#[test]
fn repeated_opening_needs_at_least_three() {
    let mk = |t: &str| TranscriptRow {
        kind: "assistant".into(),
        text: t.into(),
        ..Default::default()
    };
    let rows = vec![
        mk("Reading the same file once more today."),
        mk("Reading the same file once more today."),
    ];
    assert_eq!(repeated_opening(&rows), None);
}

#[test]
fn repeated_opening_ignores_rows_shorter_than_24_chars() {
    let mk = |t: &str| TranscriptRow {
        kind: "assistant".into(),
        text: t.into(),
        ..Default::default()
    };
    let rows = vec![mk("hi there"), mk("hi there"), mk("hi there")];
    assert_eq!(repeated_opening(&rows), None);
}

fn snap_with_rows(rows: Vec<TranscriptRow>) -> LiveSnapshot {
    LiveSnapshot {
        transcript_rows: rows,
        ..snap("s1")
    }
}

fn sys_row(text: &str) -> TranscriptRow {
    TranscriptRow {
        kind: "system".into(),
        text: text.into(),
        ..Default::default()
    }
}

#[test]
fn pinned_and_benched_system_row_is_a_danger_signal() {
    let r = row("s1");
    let s = snap_with_rows(vec![sys_row("note: pinned model muse-1.3 is benched")]);
    let sig = live_signals(&r, Some(&s), NOW);
    assert!(sig
        .iter()
        .any(|x| x.level == SignalLevel::Danger && x.text == "pinned model is benched"));
}

#[test]
fn empty_response_system_row_is_a_danger_signal() {
    let r = row("s1");
    let s = snap_with_rows(vec![sys_row("model returned empty response twice")]);
    let sig = live_signals(&r, Some(&s), NOW);
    assert!(sig
        .iter()
        .any(|x| x.level == SignalLevel::Danger && x.text == "model returned empty responses"));
}

#[test]
fn same_sentence_system_row_is_the_stall_guard_signal() {
    let r = row("s1");
    let s = snap_with_rows(vec![sys_row("guard: same sentence repeated three times")]);
    let sig = live_signals(&r, Some(&s), NOW);
    assert!(sig
        .iter()
        .any(|x| x.level == SignalLevel::Danger && x.text == "stall guard fired"));
}

#[test]
fn stopping_to_avoid_a_loop_system_row_is_the_stall_guard_signal() {
    let r = row("s1");
    let s = snap_with_rows(vec![sys_row("stopping to avoid a loop right now")]);
    let sig = live_signals(&r, Some(&s), NOW);
    assert!(sig.iter().any(|x| x.text == "stall guard fired"));
}

#[test]
fn stalled_task_system_row_warns() {
    let r = row("s1");
    let s = snap_with_rows(vec![sys_row("task has not moved for 5 turns")]);
    let sig = live_signals(&r, Some(&s), NOW);
    assert!(sig
        .iter()
        .any(|x| x.level == SignalLevel::Warn && x.text == "a task has stalled"));
}

#[test]
fn quiet_for_180s_while_busy_warns() {
    let r = FleetRow {
        busy: true,
        last_activity: NOW - 200,
        ..row("s1")
    };
    let sig = live_signals(&r, None, NOW);
    assert!(sig
        .iter()
        .any(|x| x.level == SignalLevel::Warn && x.text.starts_with("quiet for")));
}

#[test]
fn silent_for_600s_while_busy_is_danger() {
    let r = FleetRow {
        busy: true,
        last_activity: NOW - 700,
        ..row("s1")
    };
    let sig = live_signals(&r, None, NOW);
    assert!(sig
        .iter()
        .any(|x| x.level == SignalLevel::Danger && x.text.starts_with("silent for")));
}

#[test]
fn quiet_and_silent_are_suppressed_while_waiting() {
    let r = FleetRow {
        busy: true,
        waiting: true,
        last_activity: NOW - 700,
        ..row("s1")
    };
    let sig = live_signals(&r, None, NOW);
    assert!(!sig.iter().any(|x| x.text.starts_with("silent for")));
    assert!(!sig.iter().any(|x| x.text.starts_with("quiet for")));
}

#[test]
fn context_at_80_percent_warns() {
    let r = FleetRow {
        context_tokens: 80,
        context_limit: Some(100),
        ..row("s1")
    };
    let sig = live_signals(&r, None, NOW);
    assert!(sig
        .iter()
        .any(|x| x.level == SignalLevel::Warn && x.text == "context 80% full"));
}

#[test]
fn context_below_80_percent_does_not_warn() {
    let r = FleetRow {
        context_tokens: 79,
        context_limit: Some(100),
        ..row("s1")
    };
    let sig = live_signals(&r, None, NOW);
    assert!(!sig.iter().any(|x| x.text.contains("context")));
}

fn tool_row(failed: bool) -> TranscriptRow {
    TranscriptRow {
        kind: "tool".into(),
        meta: if failed {
            Some("failed".into())
        } else {
            Some("ok".into())
        },
        ..Default::default()
    }
}

#[test]
fn three_recent_tool_failures_warns() {
    let r = row("s1");
    let s = snap_with_rows(vec![tool_row(true), tool_row(true), tool_row(true)]);
    let sig = live_signals(&r, Some(&s), NOW);
    assert!(sig
        .iter()
        .any(|x| x.level == SignalLevel::Warn && x.text == "3 recent tool failures"));
}

#[test]
fn two_recent_tool_failures_does_not_warn() {
    let r = row("s1");
    let s = snap_with_rows(vec![tool_row(true), tool_row(true), tool_row(false)]);
    let sig = live_signals(&r, Some(&s), NOW);
    assert!(!sig.iter().any(|x| x.text.contains("tool failures")));
}

#[test]
fn read_only_row_gets_an_info_signal() {
    let r = FleetRow {
        read_only: true,
        ..row("s1")
    };
    let sig = live_signals(&r, None, NOW);
    assert!(sig
        .iter()
        .any(|x| x.level == SignalLevel::Info && x.text == "read-only (no input path)"));
}

#[test]
fn terminal_row_gets_an_info_signal() {
    let r = FleetRow {
        terminal: true,
        ..row("s1")
    };
    let sig = live_signals(&r, None, NOW);
    assert!(sig
        .iter()
        .any(|x| x.level == SignalLevel::Info && x.text == "runs in a terminal"));
}

#[test]
fn signals_are_sorted_danger_first() {
    let r = row("s1");
    let mk = |t: &str| TranscriptRow {
        kind: "assistant".into(),
        text: t.into(),
        ..Default::default()
    };
    let same = "Reading the same file once more today.";
    let s = LiveSnapshot {
        context_tokens: 90,
        context_limit: Some(100),
        transcript_rows: vec![mk(same), mk(same), mk(same)],
        ..snap("s1")
    };
    let sig = live_signals(&r, Some(&s), NOW);
    assert!(sig.len() >= 2);
    assert_eq!(sig.first().unwrap().level, SignalLevel::Danger);
    assert!(sig.windows(2).all(|w| w[0].level >= w[1].level));
}

#[test]
fn stop_reason_words_maps_known_reasons() {
    assert_eq!(stop_reason_words(Some("no_output")), "no output");
    assert_eq!(stop_reason_words(Some("max_steps")), "hit the step cap");
    assert_eq!(
        stop_reason_words(Some("budget_exhausted")),
        "budget exhausted"
    );
    assert_eq!(stop_reason_words(Some("interrupted")), "interrupted");
    assert_eq!(stop_reason_words(Some("final_answer")), "answered");
    assert_eq!(stop_reason_words(Some("custom_thing")), "custom thing");
    assert_eq!(stop_reason_words(None), "failed");
}

#[test]
fn context_pct_computes_a_percentage() {
    assert_eq!(context_pct(50, Some(100)), Some(50));
    assert_eq!(context_pct(200, Some(100)), Some(100));
}

#[test]
fn context_pct_is_none_without_a_limit() {
    assert_eq!(context_pct(50, None), None);
}

#[test]
fn context_pct_is_none_with_a_zero_limit() {
    assert_eq!(context_pct(50, Some(0)), None);
}

#[test]
fn fmt_age_formats_seconds_minutes_hours_days() {
    assert_eq!(fmt_age(0), "0s");
    assert_eq!(fmt_age(45), "45s");
    assert_eq!(fmt_age(90), "1m");
    assert_eq!(fmt_age(3600), "1h");
    assert_eq!(fmt_age(3660), "1h 1m");
    assert_eq!(fmt_age(90_000), "1d");
}

#[test]
fn fmt_cost_formats_by_magnitude() {
    assert_eq!(fmt_cost(0.0), "$0");
    assert_eq!(fmt_cost(0.0034), "$0.0034");
    assert_eq!(fmt_cost(1.5), "$1.50");
    assert_eq!(fmt_cost(42.0), "$42.0");
}

#[test]
fn model_short_strips_bridge_provider_prefix() {
    assert_eq!(model_short("claude-cli::"), "claude-cli");
    assert_eq!(model_short("openai::gpt-4"), "gpt-4");
    assert_eq!(model_short("bare-id"), "bare-id");
}

#[test]
fn model_short_reports_no_model_yet() {
    assert_eq!(model_short(""), "no model yet");
    assert_eq!(model_short("—"), "no model yet");
}

#[test]
fn project_name_strips_trailing_slash() {
    assert_eq!(project_name("/home/user/project/"), "project");
}

#[test]
fn project_name_handles_windows_separators() {
    assert_eq!(project_name("C:\\Users\\name\\project"), "project");
}

#[test]
fn current_task_prefers_in_progress_over_pending() {
    let r = row("s1");
    let s = LiveSnapshot {
        tasks: vec![
            Task {
                title: "pending task".into(),
                status: "pending".into(),
                ..Default::default()
            },
            Task {
                title: "in progress task".into(),
                status: "in_progress".into(),
                ..Default::default()
            },
        ],
        ..snap("s1")
    };
    let c = live_card(&r, Some(&s), NOW);
    assert_eq!(c.current_task.as_deref(), Some("in progress task"));
}

#[test]
fn tasks_done_counts_done_status() {
    let r = row("s1");
    let s = LiveSnapshot {
        tasks: vec![
            Task {
                status: "done".into(),
                ..Default::default()
            },
            Task {
                status: "done".into(),
                ..Default::default()
            },
            Task {
                status: "pending".into(),
                ..Default::default()
            },
        ],
        ..snap("s1")
    };
    let c = live_card(&r, Some(&s), NOW);
    assert_eq!(c.tasks_done, 2);
    assert_eq!(c.tasks_total, 3);
}

#[test]
fn last_line_uses_the_streaming_edge_when_present() {
    let r = row("s1");
    let s = LiveSnapshot {
        streaming: "the reply so far, still coming in".into(),
        transcript_rows: vec![TranscriptRow {
            kind: "assistant".into(),
            text: "an older finished line".into(),
            ..Default::default()
        }],
        ..snap("s1")
    };
    let c = live_card(&r, Some(&s), NOW);
    assert!(c.streaming);
    assert_eq!(
        c.last_line.as_deref(),
        Some("the reply so far, still coming in")
    );
}

#[test]
fn last_line_uses_newest_non_system_row_without_streaming() {
    let r = row("s1");
    let s = LiveSnapshot {
        transcript_rows: vec![
            TranscriptRow {
                kind: "user".into(),
                text: "first".into(),
                ..Default::default()
            },
            TranscriptRow {
                kind: "system".into(),
                text: "sys note".into(),
                ..Default::default()
            },
            TranscriptRow {
                kind: "assistant".into(),
                text: "final reply".into(),
                ..Default::default()
            },
        ],
        ..snap("s1")
    };
    let c = live_card(&r, Some(&s), NOW);
    assert!(!c.streaming);
    assert_eq!(c.last_line.as_deref(), Some("final reply"));
}

#[test]
fn past_card_title_falls_back_to_previews_first_line() {
    let p = PastRow {
        id: "p1".into(),
        title: "".into(),
        preview: Some("First line of preview\nsecond line".into()),
        ..Default::default()
    };
    let c = past_card(&p);
    assert_eq!(c.title, "First line of preview");
}

#[test]
fn projects_are_ordered_most_cards_first_then_alphabetically() {
    let cards = vec![
        live_card(
            &FleetRow {
                cwd: "/proj/b".into(),
                ..row("b1")
            },
            None,
            NOW,
        ),
        live_card(
            &FleetRow {
                cwd: "/proj/b".into(),
                ..row("b2")
            },
            None,
            NOW,
        ),
        live_card(
            &FleetRow {
                cwd: "/proj/a".into(),
                ..row("a1")
            },
            None,
            NOW,
        ),
    ];
    assert_eq!(projects(&cards), vec!["b".to_string(), "a".to_string()]);
}

// ============================================================ state: rebuild / selection

#[test]
fn fleet_then_snapshot_builds_cards() {
    let mut a = app();
    a.apply(BoardEvent::Fleet(vec![row("s1")]));
    assert!(a.card("s1").is_some());
    a.apply(BoardEvent::Snapshot(
        "s1".into(),
        LiveSnapshot {
            busy: true,
            ..snap("s1")
        },
    ));
    assert!(a.card("s1").unwrap().busy);
}

#[test]
fn selection_survives_a_rebuild() {
    let mut a = app();
    a.apply(BoardEvent::Fleet(vec![row("s1"), row("s2")]));
    a.select("s2");
    a.apply(BoardEvent::Fleet(vec![row("s1"), row("s2")]));
    assert_eq!(a.selected_card().unwrap().id, "s2");
}

#[test]
fn default_selection_is_the_first_attention_card() {
    let mut a = app();
    a.apply(BoardEvent::Fleet(vec![
        row("busy1"),
        FleetRow {
            waiting: true,
            ..row("att1")
        },
    ]));
    assert_eq!(a.selected_card().unwrap().id, "att1");
}

#[test]
fn selection_falls_through_when_attention_is_empty() {
    let mut a = app();
    a.apply(BoardEvent::Fleet(vec![FleetRow {
        busy: true,
        ..row("work1")
    }]));
    assert_eq!(a.selected_card().unwrap().id, "work1");
    assert_eq!(a.selected_card().unwrap().column, Column::Working);
}

#[test]
fn a_snapshot_with_an_older_revision_is_ignored() {
    let mut a = app();
    a.apply(BoardEvent::Fleet(vec![row("s1")]));
    a.apply(BoardEvent::Snapshot(
        "s1".into(),
        LiveSnapshot {
            title: "first".into(),
            revision: Some(5),
            ..snap("s1")
        },
    ));
    a.apply(BoardEvent::Snapshot(
        "s1".into(),
        LiveSnapshot {
            title: "second".into(),
            revision: Some(3),
            ..snap("s1")
        },
    ));
    assert_eq!(a.card("s1").unwrap().title, "first");
}

#[test]
fn a_closed_snapshot_keeps_the_card_but_stops_watching_until_the_next_fleet() {
    let mut a = app();
    a.apply(BoardEvent::Fleet(vec![row("s1")]));
    assert_eq!(a.watch_ids(), vec!["s1".to_string()]);
    a.apply(BoardEvent::Snapshot(
        "s1".into(),
        LiveSnapshot {
            closed: true,
            ..snap("s1")
        },
    ));
    // The daemon still lists it, so the board still shows it — from the fleet row's own state.
    assert!(a.card("s1").is_some());
    assert!(a.watch_ids().is_empty());
    a.apply(BoardEvent::Fleet(vec![row("s1")]));
    assert_eq!(a.watch_ids(), vec!["s1".to_string()]);
}

#[test]
fn session_closed_drops_the_snapshot_and_pauses_the_socket_but_never_hides_the_card() {
    let mut a = app();
    a.apply(BoardEvent::Fleet(vec![row("s1")]));
    a.apply(BoardEvent::Snapshot("s1".into(), snap("s1")));
    a.apply(BoardEvent::SessionClosed("s1".into()));
    assert!(a.card("s1").is_some());
    assert!(!a.snapshots.contains_key("s1"));
    assert!(a.watch_ids().is_empty());
    // Only the daemon dropping the row removes the card.
    a.apply(BoardEvent::Fleet(vec![]));
    assert!(a.card("s1").is_none());
}

#[test]
fn fleet_drops_snapshots_for_ids_that_vanished() {
    let mut a = app();
    a.apply(BoardEvent::Fleet(vec![row("s1"), row("s2")]));
    a.apply(BoardEvent::Snapshot("s1".into(), snap("s1")));
    a.apply(BoardEvent::Snapshot("s2".into(), snap("s2")));
    a.apply(BoardEvent::Fleet(vec![row("s1")]));
    assert!(a.snapshots.contains_key("s1"));
    assert!(!a.snapshots.contains_key("s2"));
}

#[test]
fn move_in_column_clamps_at_both_ends() {
    let mut a = app();
    a.apply(BoardEvent::Fleet(vec![
        FleetRow {
            busy: true,
            last_activity: NOW,
            ..row("c0")
        },
        FleetRow {
            busy: true,
            last_activity: NOW - 1,
            ..row("c1")
        },
        FleetRow {
            busy: true,
            last_activity: NOW - 2,
            ..row("c2")
        },
    ]));
    a.select("c0");
    a.move_in_column(-5);
    assert_eq!(a.selected_card().unwrap().id, "c0");
    a.move_in_column(5);
    assert_eq!(a.selected_card().unwrap().id, "c2");
}

#[test]
fn move_column_lands_on_the_card_at_the_same_height() {
    let mut a = app();
    a.apply(BoardEvent::Fleet(vec![
        FleetRow {
            waiting: true,
            last_activity: NOW,
            ..row("a0")
        },
        FleetRow {
            waiting: true,
            last_activity: NOW - 1,
            ..row("a1")
        },
        FleetRow {
            waiting: true,
            last_activity: NOW - 2,
            ..row("a2")
        },
        FleetRow {
            busy: true,
            last_activity: NOW,
            ..row("w0")
        },
        FleetRow {
            busy: true,
            last_activity: NOW - 1,
            ..row("w1")
        },
        FleetRow {
            busy: true,
            last_activity: NOW - 2,
            ..row("w2")
        },
    ]));
    a.select("w1");
    a.move_column(-1);
    assert_eq!(a.selected_card().unwrap().id, "a1");
}

#[test]
fn move_column_clamps_to_the_last_card_of_a_shorter_column() {
    let mut a = app();
    a.apply(BoardEvent::Fleet(vec![
        FleetRow {
            waiting: true,
            last_activity: NOW,
            ..row("a0")
        },
        FleetRow {
            waiting: true,
            last_activity: NOW - 1,
            ..row("a1")
        },
        FleetRow {
            busy: true,
            last_activity: NOW,
            ..row("w0")
        },
        FleetRow {
            busy: true,
            last_activity: NOW - 1,
            ..row("w1")
        },
        FleetRow {
            busy: true,
            last_activity: NOW - 2,
            ..row("w2")
        },
    ]));
    a.select("w2");
    a.move_column(-1);
    assert_eq!(a.selected_card().unwrap().id, "a1");
}

#[test]
fn move_column_skips_empty_columns() {
    let mut a = app();
    a.apply(BoardEvent::Fleet(vec![FleetRow {
        busy: true,
        ..row("w0")
    }]));
    a.apply(BoardEvent::Past(vec![PastRow {
        id: "d0".into(),
        ..Default::default()
    }]));
    a.select("w0");
    // Ready is empty; Done has one card; moving right must skip Ready and land in Done.
    a.move_column(1);
    assert_eq!(a.selected_card().unwrap().id, "d0");
}

#[test]
fn set_cursor_column_selects_the_first_card() {
    let mut a = app();
    a.apply(BoardEvent::Fleet(vec![
        FleetRow {
            busy: true,
            last_activity: NOW,
            ..row("w0")
        },
        FleetRow {
            busy: true,
            last_activity: NOW - 1,
            ..row("w1")
        },
    ]));
    a.set_cursor_column(Column::Working);
    assert_eq!(a.selected_card().unwrap().id, "w0");
}

#[test]
fn jump_in_column_goes_to_start_and_end() {
    let mut a = app();
    a.apply(BoardEvent::Fleet(vec![
        FleetRow {
            busy: true,
            last_activity: NOW,
            ..row("c0")
        },
        FleetRow {
            busy: true,
            last_activity: NOW - 1,
            ..row("c1")
        },
        FleetRow {
            busy: true,
            last_activity: NOW - 2,
            ..row("c2")
        },
    ]));
    a.select("c1");
    a.jump_in_column(true);
    assert_eq!(a.selected_card().unwrap().id, "c2");
    a.jump_in_column(false);
    assert_eq!(a.selected_card().unwrap().id, "c0");
}

#[test]
fn visible_window_scrolls_to_keep_the_selection_visible() {
    let mut a = app();
    let rows: Vec<FleetRow> = (0..10)
        .map(|i| FleetRow {
            busy: true,
            last_activity: NOW - i,
            ..row(&format!("c{i}"))
        })
        .collect();
    a.apply(BoardEvent::Fleet(rows));
    a.select("c5");
    let (start, end) = a.visible_window(Column::Working, 3);
    assert_eq!((start, end), (3, 6));
}

#[test]
fn visible_window_clamps_scroll_at_the_end() {
    let mut a = app();
    let rows: Vec<FleetRow> = (0..10)
        .map(|i| FleetRow {
            busy: true,
            last_activity: NOW - i,
            ..row(&format!("c{i}"))
        })
        .collect();
    a.apply(BoardEvent::Fleet(rows));
    a.select("c9");
    let (start, end) = a.visible_window(Column::Working, 3);
    assert_eq!((start, end), (7, 10));
}

#[test]
fn visible_window_never_scrolls_past_zero_when_capacity_exceeds_len() {
    let mut a = app();
    a.apply(BoardEvent::Fleet(vec![
        FleetRow {
            busy: true,
            ..row("c0")
        },
        FleetRow {
            busy: true,
            ..row("c1")
        },
    ]));
    let (start, end) = a.visible_window(Column::Working, 5);
    assert_eq!((start, end), (0, 2));
}

#[test]
fn project_filter_keeps_only_matching_cards() {
    let mut a = app();
    a.apply(BoardEvent::Fleet(vec![
        FleetRow {
            cwd: "/proj/one".into(),
            ..row("s1")
        },
        FleetRow {
            cwd: "/proj/two".into(),
            ..row("s2")
        },
    ]));
    a.set_project_filter(Some("two".into()));
    assert_eq!(a.cards.len(), 1);
    assert_eq!(a.cards[0].id, "s2");
}

#[test]
fn text_query_filters_cards() {
    let mut a = app();
    a.apply(BoardEvent::Fleet(vec![
        FleetRow {
            title: "fix the parser".into(),
            ..row("s1")
        },
        FleetRow {
            title: "write docs".into(),
            ..row("s2")
        },
    ]));
    a.query = "parser".into();
    a.rebuild();
    assert_eq!(a.cards.len(), 1);
    assert_eq!(a.cards[0].id, "s1");
}

#[test]
fn cycle_project_filter_walks_most_cards_first_then_back_to_none() {
    let mut a = app();
    a.apply(BoardEvent::Fleet(vec![
        FleetRow {
            cwd: "/proj/b".into(),
            ..row("b1")
        },
        FleetRow {
            cwd: "/proj/b".into(),
            ..row("b2")
        },
        FleetRow {
            cwd: "/proj/a".into(),
            ..row("a1")
        },
    ]));
    assert_eq!(a.project_filter(), None);
    a.cycle_project_filter();
    assert_eq!(a.project_filter(), Some("b"));
    a.cycle_project_filter();
    assert_eq!(a.project_filter(), Some("a"));
    a.cycle_project_filter();
    assert_eq!(a.project_filter(), None);
}

#[test]
fn esc_clears_the_query_before_the_project_filter() {
    let mut a = app_with_selected("s1");
    a.query = "foo".into();
    a.project_filter = Some("bar".into());
    handle_key(&mut a, key(KeyCode::Esc));
    assert_eq!(a.query, "");
    assert_eq!(a.project_filter, Some("bar".to_string()));
    handle_key(&mut a, key(KeyCode::Esc));
    assert_eq!(a.project_filter, None);
}

// ============================================================ state: detail

#[test]
fn open_detail_asks_for_detail_once_per_session() {
    let mut a = app_with_selected("s1");
    let actions = a.open_detail();
    assert_eq!(actions, vec![BoardAction::WantDetail("s1".into())]);
    a.close_detail();
    let actions = a.open_detail();
    assert!(actions.is_empty());
}

#[test]
fn refresh_clears_detail_requested_so_it_asks_again() {
    let mut a = app_with_selected("s1");
    a.open_detail();
    let out = handle_key(&mut a, key(KeyCode::Char('R')));
    assert!(out.contains(&BoardAction::Refresh));
    assert!(out.contains(&BoardAction::WantDetail("s1".into())));
}

#[test]
fn detail_actions_for_a_past_card_emits_nothing() {
    let mut a = app();
    a.apply(BoardEvent::Past(vec![PastRow {
        id: "p1".into(),
        ..Default::default()
    }]));
    assert!(a.detail_actions_for("p1").is_empty());
}

// ============================================================ actions: permission / question

#[test]
fn answer_permission_emits_the_allow_json_only_when_pending() {
    let mut a = app_with_selected("s1");
    a.snapshots.insert(
        "s1".into(),
        LiveSnapshot {
            permission_prompt: Some("allow write?".into()),
            prompt_seq: 42,
            ..snap("s1")
        },
    );
    let actions = a.answer_permission(true);
    assert_eq!(
        actions,
        vec![BoardAction::Input(
            "s1".into(),
            serde_json::json!({ "kind": "allow", "yes": true, "seq": 42 })
        )]
    );
}

#[test]
fn answer_permission_does_nothing_without_a_pending_prompt() {
    let mut a = app_with_selected("s1");
    assert!(a.answer_permission(true).is_empty());
}

#[test]
fn answer_option_emits_the_answer_json() {
    let mut a = app_with_selected("s1");
    a.snapshots.insert(
        "s1".into(),
        LiveSnapshot {
            question: Some("pick one".into()),
            question_options: vec![
                QOption {
                    label: "one".into(),
                    ..Default::default()
                },
                QOption {
                    label: "two".into(),
                    ..Default::default()
                },
            ],
            prompt_seq: 7,
            ..snap("s1")
        },
    );
    let actions = a.answer_option(2);
    assert_eq!(
        actions,
        vec![BoardAction::Input(
            "s1".into(),
            serde_json::json!({ "kind": "answer", "text": "2", "seq": 7 })
        )]
    );
}

#[test]
fn answer_option_rejects_out_of_range() {
    let mut a = app_with_selected("s1");
    a.snapshots.insert(
        "s1".into(),
        LiveSnapshot {
            question: Some("pick one".into()),
            question_options: vec![QOption {
                label: "one".into(),
                ..Default::default()
            }],
            ..snap("s1")
        },
    );
    assert!(a.answer_option(0).is_empty());
    assert!(a.answer_option(2).is_empty());
}

#[test]
fn digits_do_nothing_when_no_question_is_pending() {
    let mut a = app_with_selected("s1");
    let out = handle_key(&mut a, key(KeyCode::Char('1')));
    assert!(out.is_empty());
}

// ============================================================ actions: composer

#[test]
fn p_opens_the_prompt_composer_for_a_writable_target() {
    let mut a = app_with_selected("s1");
    handle_key(&mut a, key(KeyCode::Char('p')));
    let c = a.composer.as_ref().unwrap();
    assert_eq!(c.mode, ComposerMode::Prompt);
    assert_eq!(c.target, "s1");
    assert_eq!(a.focus(), Focus::Composer);
}

#[test]
fn p_does_nothing_for_a_past_card() {
    let mut a = app();
    a.apply(BoardEvent::Past(vec![PastRow {
        id: "p1".into(),
        ..Default::default()
    }]));
    a.select("p1");
    handle_key(&mut a, key(KeyCode::Char('p')));
    assert!(a.composer.is_none());
}

#[test]
fn p_does_nothing_for_a_read_only_card() {
    let mut a = app();
    a.apply(BoardEvent::Fleet(vec![FleetRow {
        read_only: true,
        ..row("s1")
    }]));
    a.select("s1");
    handle_key(&mut a, key(KeyCode::Char('p')));
    assert!(a.composer.is_none());
}

#[test]
fn typing_and_backspace_edit_the_composer_text() {
    let mut a = app_with_selected("s1");
    handle_key(&mut a, key(KeyCode::Char('p')));
    handle_key(&mut a, key(KeyCode::Char('h')));
    handle_key(&mut a, key(KeyCode::Char('i')));
    assert_eq!(a.composer.as_ref().unwrap().text, "hi");
    handle_key(&mut a, key(KeyCode::Backspace));
    assert_eq!(a.composer.as_ref().unwrap().text, "h");
}

#[test]
fn ctrl_w_deletes_the_trailing_word() {
    let mut a = app_with_selected("s1");
    handle_key(&mut a, key(KeyCode::Char('p')));
    for ch in "foo bar".chars() {
        handle_key(&mut a, key(KeyCode::Char(ch)));
    }
    handle_key(&mut a, key_mod(KeyCode::Char('w'), KeyModifiers::CONTROL));
    assert_eq!(a.composer.as_ref().unwrap().text, "foo ");
}

#[test]
fn cursor_movement_handles_multibyte_chars() {
    let mut a = app_with_selected("s1");
    handle_key(&mut a, key(KeyCode::Char('p')));
    handle_key(&mut a, key(KeyCode::Char('h')));
    handle_key(&mut a, key(KeyCode::Char('é')));
    {
        let c = a.composer.as_ref().unwrap();
        assert_eq!(c.text, "hé");
        assert_eq!(c.cursor, 2);
    }
    handle_key(&mut a, key(KeyCode::Left));
    handle_key(&mut a, key(KeyCode::Left));
    assert_eq!(a.composer.as_ref().unwrap().cursor, 0);
    handle_key(&mut a, key(KeyCode::Right));
    assert_eq!(a.composer.as_ref().unwrap().cursor, 1);
    handle_key(&mut a, key(KeyCode::End));
    assert_eq!(a.composer.as_ref().unwrap().cursor, 2);
    handle_key(&mut a, key(KeyCode::Home));
    assert_eq!(a.composer.as_ref().unwrap().cursor, 0);
}

#[test]
fn enter_submits_the_prompt_and_closes_the_composer() {
    let mut a = app_with_selected("s1");
    handle_key(&mut a, key(KeyCode::Char('p')));
    for ch in "go".chars() {
        handle_key(&mut a, key(KeyCode::Char(ch)));
    }
    let out = handle_key(&mut a, key(KeyCode::Enter));
    assert_eq!(
        out,
        vec![BoardAction::Input(
            "s1".into(),
            serde_json::json!({ "kind": "prompt", "text": "go" })
        )]
    );
    assert!(a.composer.is_none());
}

#[test]
fn empty_prompt_text_emits_nothing_on_submit() {
    let mut a = app_with_selected("s1");
    handle_key(&mut a, key(KeyCode::Char('p')));
    let out = handle_key(&mut a, key(KeyCode::Enter));
    assert!(out.is_empty());
    assert!(a.composer.is_none());
}

#[test]
fn m_prefills_the_current_model() {
    let mut a = app();
    a.apply(BoardEvent::Fleet(vec![FleetRow {
        model: "opencode::muse".into(),
        ..row("s1")
    }]));
    a.select("s1");
    handle_key(&mut a, key(KeyCode::Char('m')));
    assert_eq!(a.composer.as_ref().unwrap().text, "opencode::muse");
}

#[test]
fn m_with_empty_submit_clears_the_model_pin() {
    let mut a = app_with_selected("s1");
    handle_key(&mut a, key(KeyCode::Char('m')));
    let out = handle_key(&mut a, key(KeyCode::Enter));
    assert_eq!(
        out,
        vec![BoardAction::Input(
            "s1".into(),
            serde_json::json!({ "kind": "prompt", "text": "/model" })
        )]
    );
}

#[test]
fn m_with_text_pins_that_model() {
    let mut a = app_with_selected("s1");
    handle_key(&mut a, key(KeyCode::Char('m')));
    for ch in "foo".chars() {
        handle_key(&mut a, key(KeyCode::Char(ch)));
    }
    let out = handle_key(&mut a, key(KeyCode::Enter));
    assert_eq!(
        out,
        vec![BoardAction::Input(
            "s1".into(),
            serde_json::json!({ "kind": "prompt", "text": "/model foo" })
        )]
    );
}

#[test]
fn s_opens_the_steer_composer_and_submits_steer_json() {
    let mut a = app_with_selected("s1");
    handle_key(&mut a, key(KeyCode::Char('s')));
    assert_eq!(a.composer.as_ref().unwrap().mode, ComposerMode::Steer);
    for ch in "go".chars() {
        handle_key(&mut a, key(KeyCode::Char(ch)));
    }
    let out = handle_key(&mut a, key(KeyCode::Enter));
    assert_eq!(
        out,
        vec![BoardAction::Input(
            "s1".into(),
            serde_json::json!({ "kind": "steer", "text": "go" })
        )]
    );
}

#[test]
fn e_opens_answer_composer_only_when_a_question_is_pending() {
    let mut a = app_with_selected("s1");
    handle_key(&mut a, key(KeyCode::Char('e')));
    assert!(a.composer.is_none());

    a.snapshots.insert(
        "s1".into(),
        LiveSnapshot {
            question: Some("pick".into()),
            prompt_seq: 9,
            ..snap("s1")
        },
    );
    handle_key(&mut a, key(KeyCode::Char('e')));
    assert_eq!(
        a.composer.as_ref().unwrap().mode,
        ComposerMode::Answer { seq: 9 }
    );
}

#[test]
fn n_and_shift_n_open_new_session_with_the_worktree_flag() {
    let mut a = app_with_selected("s1");
    handle_key(&mut a, key(KeyCode::Char('N')));
    match &a.composer.as_ref().unwrap().mode {
        ComposerMode::NewSession { worktree, .. } => assert!(!worktree),
        other => panic!("unexpected mode {other:?}"),
    }
    a.cancel_composer();
    handle_key(&mut a, key(KeyCode::Char('W')));
    match &a.composer.as_ref().unwrap().mode {
        ComposerMode::NewSession { worktree, .. } => assert!(*worktree),
        other => panic!("unexpected mode {other:?}"),
    }
}

#[test]
fn tab_toggles_the_new_session_worktree_flag() {
    let mut a = app_with_selected("s1");
    handle_key(&mut a, key(KeyCode::Char('N')));
    handle_key(&mut a, key(KeyCode::Tab));
    match &a.composer.as_ref().unwrap().mode {
        ComposerMode::NewSession { worktree, .. } => assert!(*worktree),
        other => panic!("unexpected mode {other:?}"),
    }
}

#[test]
fn new_session_submit_emits_new_session_action() {
    let mut a = app_with_selected("s1");
    handle_key(&mut a, key(KeyCode::Char('N')));
    for ch in "hello".chars() {
        handle_key(&mut a, key(KeyCode::Char(ch)));
    }
    let out = handle_key(&mut a, key(KeyCode::Enter));
    assert_eq!(
        out,
        vec![BoardAction::NewSession {
            cwd: ".".into(),
            worktree: false,
            prompt: "hello".into(),
        }]
    );
}

#[test]
fn esc_cancels_the_composer_and_restores_detail_focus() {
    let mut a = app_with_selected("s1");
    a.open_detail();
    handle_key(&mut a, key(KeyCode::Char('p')));
    assert_eq!(a.focus(), Focus::Composer);
    handle_key(&mut a, key(KeyCode::Esc));
    assert!(a.composer.is_none());
    assert_eq!(a.focus(), Focus::Detail);
}

// ============================================================ actions: confirm / archive

#[test]
fn x_on_a_live_daemon_card_opens_the_archive_confirm() {
    let mut a = app_with_selected("s1");
    handle_key(&mut a, key(KeyCode::Char('x')));
    assert_eq!(
        a.confirm.as_ref().unwrap().kind,
        ConfirmKind::Archive("s1".into())
    );
    assert_eq!(a.focus(), Focus::Confirm);
}

#[test]
fn enter_on_the_confirm_archives() {
    let mut a = app_with_selected("s1");
    handle_key(&mut a, key(KeyCode::Char('x')));
    let out = handle_key(&mut a, key(KeyCode::Enter));
    assert_eq!(out, vec![BoardAction::Archive("s1".into())]);
    assert!(a.confirm.is_none());
}

#[test]
fn esc_on_the_confirm_does_nothing() {
    let mut a = app_with_selected("s1");
    handle_key(&mut a, key(KeyCode::Char('x')));
    let out = handle_key(&mut a, key(KeyCode::Esc));
    assert!(out.is_empty());
    assert!(a.confirm.is_none());
}

#[test]
fn x_on_a_past_card_toasts_instead_of_confirming() {
    let mut a = app();
    a.apply(BoardEvent::Past(vec![PastRow {
        id: "p1".into(),
        ..Default::default()
    }]));
    a.select("p1");
    handle_key(&mut a, key(KeyCode::Char('x')));
    assert!(a.confirm.is_none());
    assert!(!a.toasts.is_empty());
}

#[test]
fn x_on_a_terminal_card_toasts_instead_of_confirming() {
    let mut a = app();
    a.apply(BoardEvent::Fleet(vec![FleetRow {
        terminal: true,
        ..row("s1")
    }]));
    a.select("s1");
    handle_key(&mut a, key(KeyCode::Char('x')));
    assert!(a.confirm.is_none());
    assert!(!a.toasts.is_empty());
}

// ============================================================ actions: interrupt / mode / resume / attach

#[test]
fn interrupt_only_fires_while_busy() {
    let mut a = app();
    a.apply(BoardEvent::Fleet(vec![FleetRow {
        busy: true,
        ..row("s1")
    }]));
    a.select("s1");
    assert_eq!(
        a.interrupt_selected(),
        vec![BoardAction::Interrupt("s1".into())]
    );

    let mut b = app_with_selected("s2");
    assert!(b.interrupt_selected().is_empty());
}

#[test]
fn cycle_mode_selected_walks_the_full_cycle() {
    let cases = [
        ("default", "accept-edits"),
        ("accept-edits", "bypass"),
        ("bypass", "plan"),
        ("plan", "default"),
        ("", "accept-edits"),
    ];
    for (current, expected_next) in cases {
        let mut a = app();
        a.apply(BoardEvent::Fleet(vec![row("s1")]));
        a.select("s1");
        a.snapshots.insert(
            "s1".into(),
            LiveSnapshot {
                permission_mode: current.to_string(),
                ..snap("s1")
            },
        );
        let out = a.cycle_mode_selected();
        assert_eq!(
            out,
            vec![BoardAction::SetMode("s1".into(), expected_next.into())],
            "current={current}"
        );
    }
}

#[test]
fn cycle_mode_refuses_terminal_and_past_cards() {
    let mut a = app();
    a.apply(BoardEvent::Fleet(vec![FleetRow {
        terminal: true,
        ..row("s1")
    }]));
    a.select("s1");
    assert!(a.cycle_mode_selected().is_empty());

    let mut b = app();
    b.apply(BoardEvent::Past(vec![PastRow {
        id: "p1".into(),
        ..Default::default()
    }]));
    b.select("p1");
    assert!(b.cycle_mode_selected().is_empty());
}

#[test]
fn resume_only_fires_for_past_cards() {
    let mut a = app();
    a.apply(BoardEvent::Past(vec![PastRow {
        id: "p1".into(),
        ..Default::default()
    }]));
    a.select("p1");
    assert_eq!(a.resume_selected(), vec![BoardAction::Resume("p1".into())]);

    let mut b = app_with_selected("s1");
    assert!(b.resume_selected().is_empty());
}

#[test]
fn attach_refuses_past_and_read_only_cards() {
    let mut a = app();
    a.apply(BoardEvent::Past(vec![PastRow {
        id: "p1".into(),
        ..Default::default()
    }]));
    a.select("p1");
    assert!(a.attach_selected().is_empty());

    let mut b = app();
    b.apply(BoardEvent::Fleet(vec![FleetRow {
        read_only: true,
        ..row("s1")
    }]));
    b.select("s1");
    assert!(b.attach_selected().is_empty());

    let mut c = app_with_selected("s2");
    assert_eq!(c.attach_selected(), vec![BoardAction::Attach("s2".into())]);
}

// ============================================================ keys: navigation / focus

#[test]
fn esc_chain_leaves_the_pane_open_then_closes_it() {
    let mut a = app_with_selected("s1");
    a.open_detail();
    assert_eq!(a.focus(), Focus::Detail);
    handle_key(&mut a, key(KeyCode::Esc));
    assert_eq!(a.focus(), Focus::Board);
    assert!(a.detail_open);
    handle_key(&mut a, key(KeyCode::Esc));
    assert!(!a.detail_open);
    assert_eq!(a.focus(), Focus::Board);
}

#[test]
fn enter_on_the_board_with_the_pane_open_focuses_the_pane() {
    let mut a = app_with_selected("s1");
    a.open_detail();
    handle_key(&mut a, key(KeyCode::Esc));
    assert_eq!(a.focus(), Focus::Board);
    handle_key(&mut a, key(KeyCode::Enter));
    assert_eq!(a.focus(), Focus::Detail);
}

#[test]
fn bracket_keys_cycle_tabs_and_reset_scroll() {
    let mut a = app_with_selected("s1");
    a.detail_scroll = 5;
    a.tail_follow = false;
    handle_key(&mut a, key(KeyCode::Char(']')));
    assert_eq!(a.detail_tab, DetailTab::Tail);
    assert_eq!(a.detail_scroll, 0);
    assert!(a.tail_follow);
    handle_key(&mut a, key(KeyCode::Char('[')));
    assert_eq!(a.detail_tab, DetailTab::Overview);
}

#[test]
fn t_toggles_tool_rows() {
    let mut a = app_with_selected("s1");
    assert!(a.show_tools);
    handle_key(&mut a, key(KeyCode::Char('t')));
    assert!(!a.show_tools);
    handle_key(&mut a, key(KeyCode::Char('t')));
    assert!(a.show_tools);
}

#[test]
fn shift_f_resumes_following_the_live_tail() {
    let mut a = app_with_selected("s1");
    a.tail_follow = false;
    a.detail_tab = DetailTab::Overview;
    handle_key(&mut a, key(KeyCode::Char('F')));
    assert!(a.tail_follow);
    assert_eq!(a.detail_tab, DetailTab::Tail);
}

#[test]
fn q_and_ctrl_c_quit() {
    let mut a = app_with_selected("s1");
    assert_eq!(
        handle_key(&mut a, key(KeyCode::Char('q'))),
        vec![BoardAction::Quit]
    );
    assert_eq!(
        handle_key(&mut a, key_mod(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        vec![BoardAction::Quit]
    );
}

#[test]
fn help_opens_and_any_key_closes_it() {
    let mut a = app_with_selected("s1");
    handle_key(&mut a, key(KeyCode::Char('?')));
    assert_eq!(a.focus(), Focus::Help);
    handle_key(&mut a, key(KeyCode::Char('x')));
    assert_eq!(a.focus(), Focus::Board);
}

#[test]
fn slash_opens_filter_typing_filters_enter_keeps_esc_clears() {
    let mut a = app_with_selected("s1");
    handle_key(&mut a, key(KeyCode::Char('/')));
    assert_eq!(a.focus(), Focus::Filter);
    handle_key(&mut a, key(KeyCode::Char('a')));
    handle_key(&mut a, key(KeyCode::Char('b')));
    assert_eq!(a.query, "ab");
    handle_key(&mut a, key(KeyCode::Enter));
    assert_eq!(a.focus(), Focus::Board);
    assert_eq!(a.query, "ab");
    handle_key(&mut a, key(KeyCode::Char('/')));
    handle_key(&mut a, key(KeyCode::Esc));
    assert_eq!(a.query, "");
    assert_eq!(a.focus(), Focus::Board);
}

// ============================================================ mouse

#[test]
fn clicking_a_card_selects_it_then_a_second_click_opens_the_pane() {
    let mut a = app();
    a.apply(BoardEvent::Fleet(vec![row("s1"), row("s2")]));
    a.select("s2");
    a.hits
        .push((Rect::new(0, 0, 10, 1), Hit::Card("s1".into())));
    click_at(&mut a, 2, 0);
    assert_eq!(a.selected_card().unwrap().id, "s1");
    assert!(!a.detail_open);
    let out = click_at(&mut a, 2, 0);
    assert!(out.contains(&BoardAction::WantDetail("s1".into())));
    assert!(a.detail_open);
}

#[test]
fn clicking_a_column_header_moves_the_cursor_column() {
    let mut a = app();
    a.apply(BoardEvent::Fleet(vec![FleetRow {
        busy: true,
        ..row("s1")
    }]));
    a.hits
        .push((Rect::new(0, 0, 10, 1), Hit::ColumnHeader(Column::Working)));
    click_at(&mut a, 1, 0);
    assert_eq!(a.selected_card().unwrap().id, "s1");
}

#[test]
fn clicking_the_allow_button_emits_the_allow_json() {
    let mut a = app_with_selected("s1");
    a.snapshots.insert(
        "s1".into(),
        LiveSnapshot {
            permission_prompt: Some("ok?".into()),
            prompt_seq: 1,
            ..snap("s1")
        },
    );
    a.hits
        .push((Rect::new(0, 0, 10, 1), Hit::Button(Button::Allow)));
    let out = click_at(&mut a, 1, 0);
    assert_eq!(
        out,
        vec![BoardAction::Input(
            "s1".into(),
            serde_json::json!({ "kind": "allow", "yes": true, "seq": 1 })
        )]
    );
}

#[test]
fn clicking_close_detail_closes_the_pane() {
    let mut a = app_with_selected("s1");
    a.open_detail();
    a.hits.push((Rect::new(0, 0, 10, 1), Hit::CloseDetail));
    click_at(&mut a, 1, 0);
    assert!(!a.detail_open);
}

#[test]
fn wheel_over_cards_moves_the_selection() {
    let mut a = app();
    a.apply(BoardEvent::Fleet(vec![
        FleetRow {
            busy: true,
            last_activity: NOW,
            ..row("c0")
        },
        FleetRow {
            busy: true,
            last_activity: NOW - 1,
            ..row("c1")
        },
    ]));
    a.select("c0");
    a.hits
        .push((Rect::new(0, 0, 10, 5), Hit::Card("c0".into())));
    scroll_at(&mut a, 1, 1, true);
    assert_eq!(a.selected_card().unwrap().id, "c1");
}

#[test]
fn clicks_while_help_is_open_just_close_it() {
    let mut a = app_with_selected("s1");
    a.focus = Focus::Help;
    let out = click_at(&mut a, 0, 0);
    assert!(out.is_empty());
    assert_eq!(a.focus(), Focus::Board);
}

// ============================================================ toasts / totals / animation

#[test]
fn toasts_are_capped_at_four() {
    let mut a = app();
    for i in 0..5 {
        a.toast(ToastLevel::Info, format!("toast {i}"));
    }
    assert_eq!(a.toasts.len(), 4);
    assert_eq!(a.toasts.front().unwrap().text, "toast 1");
}

#[test]
fn toasts_expire_after_toast_ticks_and_errors_last_twice_as_long() {
    let mut a = app();
    a.toast(ToastLevel::Info, "info");
    a.toast(ToastLevel::Error, "error");
    for _ in 0..state::TOAST_TICKS {
        a.apply(BoardEvent::Tick);
    }
    assert!(a.toasts.iter().all(|t| t.text != "info"));
    assert!(a.toasts.iter().any(|t| t.text == "error"));
}

#[test]
fn needs_animation_is_true_with_a_busy_card_after_the_flash_fades() {
    let mut a = app();
    a.apply(BoardEvent::Fleet(vec![FleetRow {
        busy: true,
        ..row("s1")
    }]));
    for _ in 0..(state::FLASH_TICKS + 1) {
        a.apply(BoardEvent::Tick);
    }
    assert!(a.needs_animation());
}

#[test]
fn needs_animation_is_true_with_a_toast() {
    let mut a = app();
    a.toast(ToastLevel::Info, "hi");
    assert!(a.needs_animation());
}

#[test]
fn totals_sum_live_cost_and_running_subagents() {
    let mut a = app();
    a.apply(BoardEvent::Fleet(vec![row("s1"), row("s2")]));
    a.apply(BoardEvent::Snapshot(
        "s1".into(),
        LiveSnapshot {
            cost_usd: 1.5,
            subagents: vec![
                Subagent {
                    done: false,
                    ..Default::default()
                },
                Subagent {
                    done: true,
                    ..Default::default()
                },
            ],
            ..snap("s1")
        },
    ));
    a.apply(BoardEvent::Snapshot(
        "s2".into(),
        LiveSnapshot {
            cost_usd: 2.5,
            ..snap("s2")
        },
    ));
    let t = a.totals();
    assert_eq!(t.cost_usd, 4.0);
    assert_eq!(t.subagents, 1);
    assert_eq!(t.sessions, 2);
}

#[test]
fn watch_ids_excludes_read_only_and_closed_sessions() {
    let mut a = app();
    a.apply(BoardEvent::Fleet(vec![
        row("s1"),
        FleetRow {
            read_only: true,
            ..row("s2")
        },
        row("s3"),
    ]));
    a.apply(BoardEvent::SessionClosed("s3".into()));
    let ids = a.watch_ids();
    assert_eq!(ids, vec!["s1".to_string()]);
}

#[test]
fn is_new_is_false_for_cards_present_at_tick_zero() {
    let mut a = app();
    a.apply(BoardEvent::Fleet(vec![row("s1")]));
    assert!(!a.is_new("s1"));
}

#[test]
fn is_new_is_true_for_a_card_that_appears_after_tick_zero() {
    let mut a = app();
    for _ in 0..5 {
        a.apply(BoardEvent::Tick);
    }
    a.apply(BoardEvent::Fleet(vec![row("s1")]));
    assert!(a.is_new("s1"));
}

#[test]
fn offline_connection_toasts_once_not_on_every_repeat() {
    let mut a = app();
    a.apply(BoardEvent::Connection(ConnState::Offline("down".into())));
    assert_eq!(a.toasts.len(), 1);
    a.apply(BoardEvent::Connection(ConnState::Offline("down".into())));
    assert_eq!(a.toasts.len(), 1);
}
