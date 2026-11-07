//! The TUI side of `/turns`: how this session's and this project's turns ended over the last
//! seven days, then the recent turns that stopped short. Enter on a turn says what happened and
//! what to do about it.

use forge_core::TurnEndView;
use forge_store::Store;
use forge_tui::{PickerKind, PickerRow};

const WINDOW_DAYS: i64 = 7;
const SHOWN_COUNTS: usize = 4;
const UNFINISHED_ROWS: usize = 15;

fn when(secs: i64) -> String {
    chrono::DateTime::from_timestamp(secs, 0)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| "?".into())
}

fn counts_line(counts: &[(String, i64)]) -> String {
    let total: i64 = counts.iter().map(|(_, n)| n).sum();
    if total == 0 {
        return "no turns recorded".into();
    }
    let mut parts: Vec<String> = counts
        .iter()
        .take(SHOWN_COUNTS)
        .map(|(reason, n)| format!("{n} {}", forge_core::end_label(reason)))
        .collect();
    if counts.len() > SHOWN_COUNTS {
        parts.push(format!("{} more kinds", counts.len() - SHOWN_COUNTS));
    }
    format!("{total} turns: {}", parts.join(", "))
}

pub(crate) fn rows(
    session_counts: &[(String, i64)],
    project_counts: &[(String, i64)],
    unfinished: &[forge_store::TurnRow],
    session_id: &str,
) -> Vec<PickerRow> {
    let mut rows = vec![
        PickerRow {
            id: "summary".into(),
            title: "this session".into(),
            subtitle: counts_line(session_counts),
        },
        PickerRow {
            id: "summary".into(),
            title: format!("this project, last {WINDOW_DAYS} days"),
            subtitle: counts_line(project_counts),
        },
    ];
    for row in unfinished {
        let view = TurnEndView::from_row(row);
        let mine = if row.session_id == session_id {
            "this session"
        } else {
            &row.session_id[..row.session_id.len().min(8)]
        };
        rows.push(PickerRow {
            id: format!("turn:{}", row.id),
            title: format!("{}  {}", when(row.started_at), view.label),
            subtitle: format!(
                "{}  ·  {}  ·  {mine}",
                view.advice,
                view.shape_label.as_deref().unwrap_or("-")
            ),
        });
    }
    if unfinished.is_empty() {
        rows.push(PickerRow {
            id: "summary".into(),
            title: "every turn in the window finished".into(),
            subtitle: String::new(),
        });
    }
    rows
}

pub(crate) fn open(app: &mut forge_tui::App, store: &Store, session_id: &str) {
    let since = chrono::Utc::now().timestamp() - WINDOW_DAYS * 86_400;
    let session_counts = store
        .turn_end_counts(0, Some(session_id))
        .unwrap_or_default();
    let cwd = store.session_cwd(session_id).ok().flatten();
    let (project_counts, unfinished) = match cwd.as_deref() {
        Some(cwd) => (
            store
                .project_turn_end_counts(since, cwd)
                .unwrap_or_default(),
            store
                .project_unfinished_turns(since, cwd, UNFINISHED_ROWS)
                .unwrap_or_default(),
        ),
        None => (Vec::new(), Vec::new()),
    };
    app.picker.open_with(
        PickerKind::Turns,
        "turn endings - how turns ended, and what to do about the ones that stopped short",
        rows(&session_counts, &project_counts, &unfinished, session_id),
    );
}

pub(crate) fn accept(app: &mut forge_tui::App, row: &PickerRow) {
    let Some(id) = row.id.strip_prefix("turn:") else {
        return;
    };
    app.note(&format!(
        "turn {}: {}. {}",
        id,
        row.title.trim(),
        row.subtitle.replace("  ·  ", " - ")
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(id: i64, session: &str, reason: &str, shape: &str) -> forge_store::TurnRow {
        forge_store::TurnRow {
            id,
            session_id: session.into(),
            prompt_seq: 1,
            started_at: 1_700_000_000,
            ended_at: Some(1_700_000_050),
            end_reason: Some(reason.into()),
            detail_json: None,
            last_role_shape: Some(shape.into()),
        }
    }

    #[test]
    fn counts_come_first_in_plain_words_then_the_turns_that_stopped_short() {
        let rows = rows(
            &[("final_answer".into(), 5), ("stalled".into(), 1)],
            &[("final_answer".into(), 9), ("provider_down".into(), 2)],
            &[
                turn(7, "abcdef1234", "stalled", "unanswered_tool_results"),
                turn(6, "zzzz9999", "provider_down", "unanswered_prompt"),
            ],
            "abcdef1234",
        );
        assert_eq!(rows[0].title, "this session");
        assert_eq!(rows[0].subtitle, "6 turns: 5 finished, 1 no progress");
        assert!(rows[1].subtitle.contains("2 model provider unreachable"));
        assert_eq!(rows[2].id, "turn:7");
        assert!(rows[2].title.ends_with("no progress"));
        assert!(rows[2].subtitle.contains("send continue"));
        assert!(rows[2].subtitle.ends_with("this session"));
        assert!(rows[3].subtitle.ends_with("zzzz9999"));
        for row in &rows {
            let all = format!("{} {}", row.title, row.subtitle);
            assert!(
                !all.contains("final_answer") && !all.contains("stalled"),
                "{all}"
            );
        }
    }

    #[test]
    fn an_all_finished_window_says_so() {
        let rows = rows(
            &[("final_answer".into(), 3)],
            &[("final_answer".into(), 3)],
            &[],
            "s",
        );
        assert_eq!(
            rows.last().unwrap().title,
            "every turn in the window finished"
        );
    }

    #[test]
    fn open_lists_the_session_and_its_project() {
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/work/a", "default").unwrap();
        let id = store.open_turn(&sid, 1).unwrap();
        store.close_turn(id, "stalled", None).unwrap();
        let mut app = forge_tui::App::default();
        open(&mut app, &store, &sid);
        assert!(app.picker.open);
    }
}
