use super::*;
use crate::app::tool_cards::Member;
use forge_types::FileDiff;

fn done(name: &str, args: &str) -> ToolCard {
    let mut c = ToolCard::new(1, name.to_string(), args.to_string());
    c.status = CardStatus::Ok;
    c.ms = Some(100);
    c
}

fn failed(name: &str, args: &str) -> ToolCard {
    let mut c = done(name, args);
    c.status = CardStatus::Failed;
    c
}

fn change(name: &str, path: &str, kind: DiffKind, old: Option<&str>, new: &str) -> ToolCard {
    let mut c = done(name, &format!(r#"{{"path":"{path}"}}"#));
    c.diff = Some(FileDiff {
        path: path.into(),
        kind,
        old: old.map(str::to_string),
        new: Some(new.to_string()),
        lang: None,
        binary: false,
    });
    c
}

fn text(cards: &[ToolCard]) -> String {
    summary_text(&summarize(cards))
}

fn row_text(line: &TextLine<'_>) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

#[test]
fn categories_aggregate_in_order_of_first_appearance_with_plurals() {
    let cards = [
        done("read_file", r#"{"path":"a.rs"}"#),
        done("grep", r#"{"pattern":"fn main"}"#),
        done("shell", r#"{"command":"ls"}"#),
        done("read_file", r#"{"path":"b.rs"}"#),
        done("glob", r#"{"pattern":"**/*.rs"}"#),
        done("shell", r#"{"command":"pwd"}"#),
        done("read_file", r#"{"path":"c.rs"}"#),
        done("shell", r#"{"command":"date"}"#),
        done("shell", r#"{"command":"id"}"#),
    ];
    assert_eq!(
        text(&cards),
        "Read 3 files, searched 2 patterns, ran 4 shell commands"
    );
}

#[test]
fn one_of_a_kind_is_singular() {
    let cards = [
        done("list_dir", r#"{"path":"src"}"#),
        done("shell", r#"{"command":"ls"}"#),
        done("shell", r#"{"command":"pwd"}"#),
    ];
    assert_eq!(text(&cards), "Listed 1 directory, ran 2 shell commands");
    let cards = [
        done("list_dir", r#"{"path":"src"}"#),
        done("list_dir", r#"{"path":"docs"}"#),
        done("web_search", r#"{"query":"x"}"#),
    ];
    assert_eq!(text(&cards), "Listed 2 directories, ran 1 web search");
}

#[test]
fn file_changes_carry_their_combined_stat() {
    let cards = [
        change(
            "edit_file",
            "a.rs",
            DiffKind::Modified,
            Some("1\n2\n3\n"),
            "1\nX\n3\nY\n",
        ),
        change(
            "edit_file",
            "b.rs",
            DiffKind::Modified,
            Some("a\n"),
            "b\nc\n",
        ),
    ];
    let s = text(&cards);
    assert!(s.starts_with("Edited 2 files (+"), "{s}");
    let (a1, d1) = crate::render::diff_stat(cards[0].diff.as_ref().unwrap());
    let (a2, d2) = crate::render::diff_stat(cards[1].diff.as_ref().unwrap());
    assert_eq!(
        s,
        format!("Edited 2 files (+{} −{})", a1 + a2, d1 + d2),
        "{s}"
    );
}

#[test]
fn a_deletion_is_never_folded_into_edited() {
    let cards = [
        change("edit_file", "a.rs", DiffKind::Modified, Some("a\n"), "b\n"),
        change("write_file", "new.rs", DiffKind::Created, None, "x\n"),
        done("delete_file", r#"{"path":"old.rs"}"#),
    ];
    assert_eq!(
        text(&cards),
        "Edited 1 file (+1 −1), created 1 file (+1), deleted 1 file"
    );
}

#[test]
fn a_single_call_names_what_it_touched() {
    assert_eq!(
        text(&[done("shell", r#"{"command":"cargo test -p upload"}"#)]),
        "Ran cargo test -p upload"
    );
    assert_eq!(
        text(&[done("read_file", r#"{"path":"Cargo.toml"}"#)]),
        "Read Cargo.toml"
    );
    assert_eq!(
        text(&[change(
            "write_file",
            "notes.md",
            DiffKind::Created,
            None,
            "a\nb\n"
        )]),
        "Created notes.md (+2)"
    );
    // Nothing to name: the counted form.
    assert_eq!(text(&[done("run_code", r#"{"code":"1"}"#)]), "Ran 1 script");
}

#[test]
fn a_read_group_card_counts_every_read_it_holds() {
    let mut c = done("read_file", r#"{"path":"d.rs"}"#);
    for p in ["a.rs", "b.rs", "c.rs"] {
        c.group.push(Member {
            name: "read_file".into(),
            args: format!(r#"{{"path":"{p}"}}"#),
            summary: String::new(),
            ms: Some(50),
        });
    }
    let s = summarize([&c]);
    assert_eq!(summary_text(&s), "Read 4 files");
    assert_eq!(s.ms, 250);
}

#[test]
fn agents_mcp_tasks_and_unknown_tools_have_their_own_words() {
    let cards = [
        done("spawn_agents", r#"{"agents":[{"task":"a"},{"task":"b"}]}"#),
        done("update_tasks", r#"{"tasks":[]}"#),
        done("mcp__github__list_issues", r#"{}"#),
        done("mcp__github__get_issue", r#"{}"#),
        done("device", r#"{"action":"tap"}"#),
    ];
    assert_eq!(
        text(&cards),
        "Ran 2 agents, updated the task list, called github 2 times, called device 1 time"
    );
}

#[test]
fn failures_and_running_calls_are_counted_not_hidden() {
    let mut running = done("shell", r#"{"command":"sleep 9"}"#);
    running.status = CardStatus::Running;
    let cards = [
        done("shell", r#"{"command":"ls"}"#),
        failed("shell", r#"{"command":"false"}"#),
        running,
    ];
    let s = summarize(&cards);
    assert_eq!((s.finished, s.failed, s.running), (2, 1, 1));
    assert_eq!(summary_text(&s), "Ran 2 shell commands");
    let row = row_text(&summary_row(&s, RowState::default()));
    assert!(row.contains("1 failed"), "{row}");
    assert!(row.contains('▸'), "collapsed groups offer to open: {row}");
}

#[test]
fn the_row_shows_the_duration_and_flips_its_triangle_when_open() {
    let s = summarize(&[done("shell", r#"{"command":"ls"}"#)]);
    let closed = row_text(&summary_row(&s, RowState::default()));
    assert!(
        closed.contains("▸ Ran ls") && closed.contains("100ms"),
        "{closed}"
    );
    let open = row_text(&summary_row(
        &s,
        RowState {
            expanded: true,
            ..RowState::default()
        },
    ));
    assert!(open.contains("▾ Ran ls"), "{open}");
}

#[test]
fn a_focused_row_says_which_keys_work() {
    let s = summarize(&[done("shell", r#"{"command":"ls"}"#)]);
    let row = row_text(&summary_row(
        &s,
        RowState {
            focused: true,
            ..RowState::default()
        },
    ));
    assert!(
        row.contains("Enter expand") && row.contains("Esc done"),
        "{row}"
    );
    assert!(
        row.contains("❯ Ran ls"),
        "focus shows the list caret: {row}"
    );
}

#[test]
fn the_failure_count_survives_a_narrow_terminal() {
    let cards = [
        done("list_dir", r#"{"path":"."}"#),
        done("glob", r#"{"pattern":"*"}"#),
        done("read_file", r#"{"path":"a"}"#),
        failed("shell", r#"{"command":"exit 3"}"#),
        done("shell", r#"{"command":"ls"}"#),
    ];
    let row = summary_row(&summarize(&cards), RowState::default());
    let fitted = crate::transcript::wrap_lines(&[row], 40);
    assert_eq!(fitted.len(), 1, "one row");
    let s = row_text(&fitted[0]);
    assert!(s.contains("1 failed"), "{s}");
    assert!(s.contains('…'), "the summary is what gets cut: {s}");
}

#[test]
fn indenting_keeps_the_rail_in_front() {
    let row = indented(rail::text_row(
        crate::theme::styles::RailKind::Settled(2),
        "x",
        styles::fg(),
    ));
    let s = row_text(&row);
    assert!(s.starts_with(" ▏   "), "{s:?}");
}

#[test]
fn a_narrow_row_gives_the_duration_up_but_keeps_the_failures() {
    let cards = [
        done("shell", r#"{"command":"ls"}"#),
        failed("shell", r#"{"command":"false"}"#),
    ];
    let row = row_text(&summary_row(
        &summarize(&cards),
        RowState {
            compact: true,
            ..RowState::default()
        },
    ));
    assert!(row.contains("1 failed") && !row.contains("200ms"), "{row}");
}
