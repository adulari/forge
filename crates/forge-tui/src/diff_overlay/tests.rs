use ratatui::backend::TestBackend;
use ratatui::Terminal;

use super::*;
use crate::KeyKind;

const GIT: &str = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,3 +1,3 @@\n one\n-two\n+TWO\n three\n@@ -20,2 +20,3 @@\n x\n+added\n y\ndiff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -1 +1 @@\n-old\n+new\ndiff --git a/c.rs b/c.rs\n--- a/c.rs\n+++ b/c.rs\n@@ -1 +1 @@\n-c1\n+c2\n";

fn opened() -> DiffOverlay {
    let mut o = DiffOverlay::default();
    o.open_with(Vec::new());
    o.set_tree(Ok(parse_unified(GIT)));
    o
}

fn draw(o: &DiffOverlay, w: u16, h: u16) -> String {
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(|f| render_diff_overlay(f, o)).unwrap();
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
fn opens_on_the_tree_when_the_turn_changed_nothing() {
    let o = opened();
    assert_eq!(o.source_index(), TREE);
    assert_eq!(o.files().len(), 3);
}

#[test]
fn opens_on_the_turn_when_it_changed_files() {
    let mut o = DiffOverlay::default();
    o.open_with(parse_unified(GIT));
    assert_eq!(o.source_index(), TURN);
}

#[test]
fn files_and_hunks_are_navigated_with_the_keys() {
    let mut o = opened();
    draw(&o, 120, 8);
    o.key(KeyKind::Char(']'));
    assert!(o.scroll() > 0, "second hunk is further down");
    let second = o.scroll();
    o.key(KeyKind::Char(']'));
    assert_eq!(o.scroll(), second, "no hunk after the last");
    o.key(KeyKind::Char('['));
    assert_eq!(o.scroll(), 0);
    o.key(KeyKind::Char('n'));
    assert_eq!(o.file_index(), 1);
    o.key(KeyKind::Char('p'));
    o.key(KeyKind::Char('p'));
    assert_eq!(o.file_index(), 2, "wraps");
}

#[test]
fn marking_a_file_reviewed_moves_on_and_counts() {
    let mut o = opened();
    o.key(KeyKind::Char('m'));
    assert_eq!(o.file_index(), 1);
    assert_eq!(o.reviewed_count(), 1);
    o.key(KeyKind::Char('p'));
    o.key(KeyKind::Char('m'));
    assert_eq!(o.reviewed_count(), 0, "the same key unmarks");
    assert_eq!(o.file_index(), 0, "unmarking stays put");
}

#[test]
fn a_reviewed_mark_does_not_survive_a_changed_file() {
    let mut o = opened();
    o.key(KeyKind::Char('m'));
    assert_eq!(o.reviewed_count(), 1);
    o.set_tree(Ok(parse_unified(&GIT.replace("+TWO", "+TWO!"))));
    assert_eq!(o.reviewed_count(), 0);
    o.set_tree(Ok(parse_unified(GIT)));
    assert_eq!(o.reviewed_count(), 1, "back to what was reviewed");
}

#[test]
fn d_switches_source_and_asks_for_git_only_for_the_tree() {
    let mut o = opened();
    assert_eq!(o.key(KeyKind::Char('d')), DiffAction::None);
    assert_eq!(o.source_index(), TURN);
    assert_eq!(o.key(KeyKind::Char('d')), DiffAction::Refresh);
    assert!(o.loading);
}

#[test]
fn escape_closes() {
    let mut o = opened();
    o.key(KeyKind::Esc);
    assert!(!o.open);
}

#[test]
fn the_wide_view_lists_files_with_their_marks() {
    let mut o = opened();
    o.key(KeyKind::Char('m'));
    let text = draw(&o, 120, 20);
    assert!(
        text.contains("a.rs") && text.contains("b.rs") && text.contains("c.rs"),
        "{text}"
    );
    assert!(text.contains("reviewed 1/3"), "{text}");
    assert!(text.contains("@@ -1 +1 @@"), "{text}");
    assert!(text.contains("unified"), "{text}");
}

#[test]
fn the_phone_width_view_names_the_file_above_the_diff() {
    let o = opened();
    let text = draw(&o, 60, 16);
    assert!(text.contains("a.rs"), "{text}");
    assert!(text.contains("two") && text.contains("TWO"), "{text}");
    assert!(text.contains("reviewed 0/3"), "{text}");
}

#[test]
fn split_pairs_a_deletion_with_its_replacement_on_one_row() {
    let mut o = opened();
    o.key(KeyKind::Char('v'));
    let text = draw(&o, 120, 20);
    let row = text
        .lines()
        .find(|l| l.contains("two"))
        .expect("a row with the old line");
    assert!(row.contains("TWO"), "old and new share a row: {row}");
    let (rows, _) = layout(o.current().unwrap(), true);
    let (unified, _) = layout(o.current().unwrap(), false);
    assert!(rows.len() < unified.len());
}

#[test]
fn empty_sources_say_why() {
    let mut o = DiffOverlay::default();
    o.open_with(Vec::new());
    assert!(draw(&o, 80, 10).contains("reading git"));
    o.set_tree(Err("not a git repository".into()));
    assert!(draw(&o, 80, 10).contains("not a git repository"));
    o.set_tree(Ok(Vec::new()));
    assert!(draw(&o, 80, 10).contains("no uncommitted changes"));
}

#[test]
fn a_binary_files_mark_is_dropped_when_its_blob_changes() {
    let bin = |blob: &str| {
        format!("diff --git a/i.png b/i.png\nindex {blob} 100644\nBinary files a/i.png and b/i.png differ\n")
    };
    let mut o = DiffOverlay::default();
    o.open_with(Vec::new());
    o.set_tree(Ok(parse_unified(&bin("111..222"))));
    o.key(KeyKind::Char('m'));
    assert_eq!(o.reviewed_count(), 1);
    o.set_tree(Ok(parse_unified(&bin("111..333"))));
    assert_eq!(o.reviewed_count(), 0);
}

#[test]
fn an_untracked_file_without_content_is_remarked_when_its_identity_changes() {
    let at = |identity: &str| {
        let mut f = untracked_file("big.bin", None);
        f.identity = identity.to_string();
        f
    };
    let mut o = DiffOverlay::default();
    o.open_with(Vec::new());
    o.set_tree(Ok(vec![at("10:1")]));
    o.key(KeyKind::Char('m'));
    assert_eq!(o.reviewed_count(), 1);
    o.set_tree(Ok(vec![at("10:2")]));
    assert_eq!(o.reviewed_count(), 0);
}

#[test]
fn end_reaches_the_last_line_on_a_narrow_split_terminal_with_a_note() {
    let mut text = String::from("diff --git a/n.rs b/n.rs\nsimilarity index 90%\nrename from o.rs\nrename to n.rs\n--- a/o.rs\n+++ b/n.rs\n@@ -1,30 +1,30 @@\n");
    for i in 0..29 {
        text.push_str(&format!("-old{i}\n+new{i}\n"));
    }
    text.push_str("-old29\n+LASTLINE\n");
    let mut o = DiffOverlay::default();
    o.open_with(Vec::new());
    o.set_tree(Ok(parse_unified(&text)));
    o.split = true;
    draw(&o, 60, 14);
    o.key(KeyKind::Char('G'));
    assert!(draw(&o, 60, 14).contains("LASTLINE"));
    o.key(KeyKind::Char('g'));
    o.key(KeyKind::Char(']'));
    assert_eq!(
        o.scroll(),
        0,
        "a single hunk has nothing further to jump to"
    );
}
