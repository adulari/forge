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
    c.set_diff(FileDiff {
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
        "Read 3 files, searched for 2 patterns, ran 4 shell commands"
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
    assert!(s.starts_with("Edited a.rs, b.rs (+"), "{s}");
    let (a1, d1) = crate::render::diff_stat(cards[0].diff.as_ref().unwrap());
    let (a2, d2) = crate::render::diff_stat(cards[1].diff.as_ref().unwrap());
    assert_eq!(
        s,
        format!("Edited a.rs, b.rs (+{} −{})", a1 + a2, d1 + d2),
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
        "Edited a.rs (+1 −1), created new.rs (+1), deleted old.rs"
    );
}

#[test]
fn a_change_names_up_to_two_files_and_counts_beyond() {
    let three: Vec<_> = ["a.rs", "b.rs", "c.rs"]
        .into_iter()
        .map(|f| change("edit_file", f, DiffKind::Modified, Some("a\n"), "b\n"))
        .collect();
    assert_eq!(text(&three), "Edited 3 files (+3 −3)");
    assert_eq!(text(&three[..2]), "Edited a.rs, b.rs (+2 −2)");
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
    // The failed call is not counted as run: the row says so in red instead.
    assert_eq!(summary_text(&s), "Ran ls");
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
    // Open, the call is listed under the header, so the header counts rather than repeat it.
    assert!(open.contains("▾ Ran 1 shell command"), "{open}");
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

fn read(path: &str, offset: usize) -> ToolCard {
    done(
        "read_file",
        &format!(r#"{{"path":"{path}","offset":{offset}}}"#),
    )
}

fn edit(path: &str, old: &str, new: &str) -> ToolCard {
    change("edit_file", path, DiffKind::Modified, Some(old), new)
}

#[test]
fn chunked_reads_and_repeated_edits_of_one_file_count_it_once() {
    let cards = [
        read("src/big.rs", 0),
        read("src/big.rs", 200),
        read("./src/big.rs", 400),
        done("shell", r#"{"command":"cargo check"}"#),
        edit("src/big.rs", "a\n", "b\n"),
        edit("src/big.rs", "c\n", "d\ne\n"),
        edit("src/big.rs", "f\n", "g\n"),
    ];
    assert_eq!(
        text(&cards),
        "Read 1 file, ran 1 shell command, edited src/big.rs (+4 −3)"
    );
    let reads = [read("src/big.rs", 0), read("src/big.rs", 200)];
    assert_eq!(
        text(&reads),
        "Read src/big.rs",
        "one file is named, however many slices"
    );
    let edits = [edit("a.rs", "1\n", "2\n"), edit("a.rs", "3\n", "4\n")];
    assert_eq!(text(&edits), "Edited a.rs (+2 −2)");
}

#[test]
fn the_same_pattern_searched_twice_is_one_pattern() {
    let cards = [
        done("grep", r#"{"pattern":"fn main","path":"src"}"#),
        done("grep", r#"{"pattern":"fn main","path":"tests"}"#),
        done("list_dir", r#"{"path":"src"}"#),
        done("list_dir", r#"{"path":"./src"}"#),
    ];
    assert_eq!(text(&cards), "Searched for 1 pattern, listed 1 directory");
}

#[test]
fn a_read_group_card_counts_the_distinct_files_it_holds() {
    let mut c = done("read_file", r#"{"path":"a.rs"}"#);
    for p in ["a.rs", "b.rs", "a.rs"] {
        c.group.push(Member {
            name: "read_file".into(),
            args: format!(r#"{{"path":"{p}"}}"#),
            summary: String::new(),
            ms: Some(50),
        });
    }
    assert_eq!(summary_text(&summarize([&c])), "Read 2 files");
}

#[test]
fn a_failed_call_is_never_tallied_as_done() {
    let mut no_op = done("write_file", r#"{"path":"notes.md","content":""}"#);
    no_op.summary = "unchanged".into();
    let cards = [
        failed("edit_file", r#"{"path":"sample.rs","old":"x","new":"y"}"#),
        no_op,
    ];
    let s = summarize(&cards);
    assert_eq!(summary_text(&s), "Edited notes.md");
    assert_eq!(s.failed, 1);
}

#[test]
fn a_failed_change_adds_nothing_to_the_stat() {
    let mut bad = edit("b.rs", "1\n", "2\n3\n");
    bad.status = CardStatus::Failed;
    let cards = [edit("a.rs", "1\n", "2\n"), bad];
    assert_eq!(text(&cards), "Edited a.rs (+1 −1)");
}

#[test]
fn a_group_where_nothing_succeeded_says_what_it_tried() {
    assert_eq!(
        text(&[failed(
            "edit_file",
            r#"{"path":"sample.rs","old":"x","new":"y"}"#
        )]),
        "Tried to edit sample.rs"
    );
    assert_eq!(
        text(&[
            failed("shell", r#"{"command":"false"}"#),
            failed("shell", r#"{"command":"exit 3"}"#),
        ]),
        "Tried to run 2 shell commands"
    );
    assert_eq!(
        text(&[
            failed("read_file", r#"{"path":"gone.rs"}"#),
            failed("shell", r#"{"command":"false"}"#),
        ]),
        "Tried to read 1 file, run 1 shell command"
    );
}

#[test]
fn the_stat_is_the_count_cached_when_the_diff_attached() {
    let mut c = edit("a.rs", "1\n", "2\n");
    assert_eq!(c.diff_counts, (1, 1));
    // The summary never diffs the file again: whatever was counted at attach time is used.
    c.diff_counts = (7, 3);
    assert_eq!(text(&[c]), "Edited a.rs (+7 −3)");
}

#[test]
fn a_narrow_row_switches_to_counts_before_cutting_words() {
    let cards = [
        done("list_dir", r#"{"path":"."}"#),
        done("glob", r#"{"pattern":"**/*.rs"}"#),
        done("grep", r#"{"pattern":"fn main"}"#),
        read("sample.rs", 0),
        done("shell", r#"{"command":"ls"}"#),
        failed("shell", r#"{"command":"exit 3"}"#),
        done("shell", r#"{"command":"pwd"}"#),
    ];
    let s = summarize(&cards);
    assert_eq!(
        summary_text_short(&s),
        "1 dir, 2 searches, 1 file read, 2 commands"
    );
    let row = summary_row(
        &s,
        RowState {
            compact: true,
            width: 59,
            ..RowState::default()
        },
    );
    let fitted = row_text(&crate::transcript::wrap_lines(&[row], 59)[0]);
    assert!(
        fitted.contains("1 dir, 2 searches, 1 file read, 2 commands")
            && fitted.contains("1 failed"),
        "{fitted}"
    );
    assert!(!fitted.contains('…'), "nothing is cut: {fitted}");
    // Wide enough for the sentence, the sentence it is.
    let wide = row_text(&summary_row(
        &s,
        RowState {
            width: 120,
            ..RowState::default()
        },
    ));
    assert!(
        wide.contains("Listed 1 directory, searched for 2 patterns, read 1 file, ran 2 shell"),
        "{wide}"
    );
}

#[test]
fn the_expand_hint_shows_only_when_it_fits_whole() {
    let s = summarize(&[done("shell", r#"{"command":"ls"}"#)]);
    let hint = Some("click or Alt+O to expand".to_string());
    let roomy = row_text(&summary_row(
        &s,
        RowState {
            width: 100,
            hint: hint.clone(),
            ..RowState::default()
        },
    ));
    assert!(
        roomy.contains("Ran ls  click or Alt+O to expand"),
        "{roomy}"
    );
    let tight = row_text(&summary_row(
        &s,
        RowState {
            width: 30,
            hint: hint.clone(),
            ..RowState::default()
        },
    ));
    assert!(!tight.contains("click"), "{tight}");
    let focused = row_text(&summary_row(
        &s,
        RowState {
            width: 100,
            hint,
            focused: true,
            ..RowState::default()
        },
    ));
    assert!(
        !focused.contains("click"),
        "focus shows its own keys: {focused}"
    );
}

#[test]
fn a_focused_row_keeps_its_failure_count() {
    let cards = [
        done("shell", r#"{"command":"ls"}"#),
        failed("shell", r#"{"command":"false"}"#),
    ];
    let row = row_text(&summary_row(
        &summarize(&cards),
        RowState {
            focused: true,
            ..RowState::default()
        },
    ));
    assert!(
        row.contains("1 failed") && row.contains("Enter expand"),
        "{row}"
    );
}
