use super::*;

fn diag(severity: DiagnosticSeverity, line: u32, code: &str, message: &str) -> Diagnostic {
    Diagnostic {
        severity,
        message: message.to_string(),
        line,
        character: 0,
        code: Some(code.to_string()),
    }
}

fn warn(line: u32, message: &str) -> Diagnostic {
    diag(DiagnosticSeverity::Warning, line, "w", message)
}

fn error(line: u32, message: &str) -> Diagnostic {
    diag(DiagnosticSeverity::Error, line, "e", message)
}

fn numbered(n: usize) -> String {
    (0..n).map(|i| format!("line {i}\n")).collect()
}

fn with_line(text: &str, index: usize, replacement: &str) -> String {
    text.lines()
        .enumerate()
        .map(|(i, l)| {
            if i == index {
                format!("{replacement}\n")
            } else {
                format!("{l}\n")
            }
        })
        .collect()
}

fn path() -> PathBuf {
    PathBuf::from("/repo/a.py")
}

#[test]
fn changed_ranges_finds_the_edited_lines_and_deletions() {
    let before = numbered(10);
    let after = with_line(&before, 4, "changed");
    assert_eq!(changed_ranges(Some(&before), &after), vec![4..5]);

    let deleted: String = before
        .lines()
        .enumerate()
        .filter(|(i, _)| *i != 6)
        .map(|(_, l)| format!("{l}\n"))
        .collect();
    assert_eq!(changed_ranges(Some(&before), &deleted), vec![6..6]);

    assert_eq!(changed_ranges(None, &before), vec![0..10]);
}

#[test]
fn first_touch_shows_only_what_is_near_the_edit_plus_a_total() {
    let before = numbered(100);
    let after = with_line(&before, 50, "edited");
    let current = vec![
        warn(2, "far away"),
        warn(51, "next to the edit"),
        warn(90, "also far"),
    ];
    let block = render("a.py", None, Some(&before), &after, &current).unwrap();
    assert!(block.contains("3 total"), "{block}");
    assert!(block.contains("next to the edit"), "{block}");
    assert!(!block.contains("far away"), "{block}");
    assert!(!block.contains("also far"), "{block}");
}

#[test]
fn first_touch_still_shows_an_error_anywhere_in_the_file() {
    let before = numbered(100);
    let after = with_line(&before, 50, "edited");
    let current = vec![error(5, "name `x` is not defined")];
    let block = render("a.py", None, Some(&before), &after, &current).unwrap();
    assert!(block.contains("name `x` is not defined"), "{block}");
}

#[test]
fn a_later_edit_reports_only_new_diagnostics_and_counts_resolved_ones() {
    let mut baselines = Baselines::default();
    let v0 = numbered(100);
    let v1 = with_line(&v0, 10, "first edit");
    let backlog: Vec<Diagnostic> = (20..40).map(|l| warn(l, "unknown type")).collect();
    let mut first = backlog.clone();
    first.push(warn(70, "unused import"));
    baselines.observe(path(), "a.py", Some(&v0), v1.clone(), first);

    // Insert a line at the top: every old line shifts down by one, which must not make the
    // backlog look new. The unused import is gone; one fresh warning appears.
    let v2 = format!("inserted\n{v1}");
    let mut second: Vec<Diagnostic> = backlog
        .iter()
        .map(|d| warn(d.line + 1, "unknown type"))
        .collect();
    second.push(warn(0, "brand new warning"));
    let block = baselines
        .observe(path(), "a.py", Some(&v1), v2, second)
        .unwrap();
    assert!(block.contains("1 new, 1 resolved"), "{block}");
    assert!(block.contains("brand new warning"), "{block}");
    assert!(!block.contains("unknown type"), "{block}");
}

#[test]
fn an_unchanged_backlog_after_an_edit_says_nothing() {
    let mut baselines = Baselines::default();
    let v0 = numbered(50);
    let v1 = with_line(&v0, 45, "edit one");
    let backlog: Vec<Diagnostic> = (0..10).map(|l| warn(l, "unknown type")).collect();
    baselines.observe(path(), "a.py", Some(&v0), v1.clone(), backlog.clone());
    let v2 = with_line(&v1, 46, "edit two");
    assert_eq!(
        baselines.observe(path(), "a.py", Some(&v1), v2, backlog),
        None
    );
}

#[test]
fn an_introduced_error_is_always_shown_even_past_the_cap() {
    let mut baselines = Baselines::default();
    let v0 = numbered(400);
    baselines.observe(path(), "a.py", Some(&v0), v0.clone(), vec![]);
    let v1 = with_line(&v0, 300, "broken(");
    // A flood of new warnings near the top fills the soft cap before the error is considered,
    // were order by line alone; the error must still be in the block.
    let mut current: Vec<Diagnostic> = (0..200)
        .map(|l| {
            warn(
                l,
                &format!("new warning number {l} with a reasonably long message"),
            )
        })
        .collect();
    current.push(error(300, "expected expression"));
    let block = baselines
        .observe(path(), "a.py", Some(&v0), v1, current)
        .unwrap();
    assert!(block.contains("expected expression"), "{block}");
    let first_finding = block.lines().nth(1).unwrap();
    assert!(first_finding.contains("expected expression"), "{block}");
}

#[test]
fn the_block_is_capped_with_a_more_tail() {
    let before = numbered(300);
    let after: String = before.lines().map(|l| format!("{l} edited\n")).collect();
    let current: Vec<Diagnostic> = (0..300)
        .map(|l| {
            warn(
                l,
                &format!("warning {l} with a message long enough to matter"),
            )
        })
        .collect();
    let block = render("a.py", None, Some(&before), &after, &current).unwrap();
    assert!(block.len() <= SOFT_CAP_CHARS + 120, "{} chars", block.len());
    assert!(block.contains("more (run the linter"), "{block}");
}

#[test]
fn order_is_deterministic_most_severe_and_nearest_first() {
    let before = numbered(100);
    let after = with_line(&before, 50, "edited");
    let current = vec![
        warn(53, "warning three away"),
        diag(DiagnosticSeverity::Hint, 50, "h", "hint on the line"),
        warn(50, "warning on the line"),
        error(52, "error two away"),
    ];
    let block = render("a.py", None, Some(&before), &after, &current).unwrap();
    let mut reversed = current.clone();
    reversed.reverse();
    let again = render("a.py", None, Some(&before), &after, &reversed).unwrap();
    assert_eq!(block, again, "input order must not change the block");
    let findings: Vec<&str> = block.lines().skip(1).collect();
    assert!(findings[0].contains("error two away"), "{block}");
    assert!(findings[1].contains("warning on the line"), "{block}");
    assert!(findings[2].contains("warning three away"), "{block}");
    assert!(findings[3].contains("hint on the line"), "{block}");
}

#[test]
fn a_multi_line_message_is_one_line_and_the_path_is_not_repeated() {
    let before = numbered(5);
    let after = with_line(&before, 1, "x");
    let current = vec![warn(
        1,
        "Type of \"_items\" is partially unknown\n  Type is dict",
    )];
    let block = render("inventory/store.py", None, Some(&before), &after, &current).unwrap();
    assert_eq!(block.lines().count(), 2, "{block}");
    assert!(
        block
            .lines()
            .nth(1)
            .unwrap()
            .starts_with("  L2:1 warning (w)"),
        "{block}"
    );
    assert_eq!(block.matches("inventory/store.py").count(), 1, "{block}");
}

#[test]
fn clear_forgets_every_snapshot() {
    let mut baselines = Baselines::default();
    let v0 = numbered(50);
    let backlog: Vec<Diagnostic> = (0..10).map(|l| warn(l, "unknown type")).collect();
    baselines.observe(path(), "a.py", Some(&v0), v0.clone(), backlog.clone());
    baselines.clear();
    let block = baselines
        .observe(path(), "a.py", Some(&v0), v0.clone(), backlog)
        .unwrap();
    assert!(block.contains("10 total"), "first touch again: {block}");
}
