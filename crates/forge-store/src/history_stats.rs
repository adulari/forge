//! Numbers a history row carries beyond its text: what an edit call changed and what a model
//! reply cost. Both are computed from persisted data at read time, so no migration is needed and
//! rows written before this existed report them too.

use serde_json::Value;

/// Additive facts about one history row. Every field is absent unless the store can state it.
#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize)]
pub struct RowStats {
    /// Lines an edit call added, from its full (uncapped) arguments.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adds: Option<u32>,
    /// Lines an edit call removed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dels: Option<u32>,
    /// Tokens the model reply that produced this row consumed, summed over its recorded usage.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
}

impl RowStats {
    pub(crate) fn lines(counts: Option<(u32, u32)>) -> Self {
        Self {
            adds: counts.map(|c| c.0),
            dels: counts.map(|c| c.1),
            ..Self::default()
        }
    }
}

fn line_count(text: &str) -> u32 {
    u32::try_from(text.lines().count()).unwrap_or(u32::MAX)
}

/// Lines replaced: the lines the two texts share at the start and end are not a change.
fn replaced_lines(old: &str, new: &str) -> (u32, u32) {
    let old: Vec<&str> = old.lines().collect();
    let new: Vec<&str> = new.lines().collect();
    let shared_head = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let shared_tail = old[shared_head..]
        .iter()
        .rev()
        .zip(new[shared_head..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let n = |v: &[&str]| u32::try_from(v.len() - shared_head - shared_tail).unwrap_or(u32::MAX);
    (n(&new), n(&old))
}

fn edit_pair(edit: &Value) -> (u32, u32) {
    let text = |key: &str| edit.get(key).and_then(Value::as_str).unwrap_or("");
    replaced_lines(text("old"), text("new"))
}

/// Lines an anchored `edit_file` call (`edits: [{start, end?, new} | {after, new}]`) wrote and
/// replaced. The file text is not in the call, so a replaced range counts whole.
fn anchored_lines(edits: &[Value]) -> (u32, u32) {
    let count = |n: u64| u32::try_from(n).unwrap_or(u32::MAX);
    edits.iter().fold((0, 0), |(added, removed), edit| {
        let get = |key: &str| edit.get(key).and_then(Value::as_u64);
        let written = edit
            .get("new")
            .and_then(Value::as_str)
            .map_or(0, line_count);
        let gone = get("start").map_or(0, |start| {
            count(get("end").unwrap_or(start).max(start) - start + 1)
        });
        (added + written, removed + gone)
    })
}

fn patch_lines(patch: &str) -> (u32, u32) {
    let mut adds = 0u32;
    let mut dels = 0u32;
    for line in patch.lines() {
        if line.starts_with('+') && !line.starts_with("+++") {
            adds += 1;
        } else if line.starts_with('-') && !line.starts_with("---") {
            dels += 1;
        }
    }
    (adds, dels)
}

/// A question form is small and its row is only readable whole (the decision record needs every
/// question and option), so it is not capped like a file body would be.
const MAX_FORM_ARGS_CHARS: usize = 6_000;

pub(crate) fn is_question_form(name: &str) -> bool {
    name.strip_prefix("mcp__forge__").unwrap_or(name) == "ask_user"
}

pub(crate) fn form_args(args: &Value) -> String {
    let line = serde_json::to_string(args).unwrap_or_else(|_| "{}".to_string());
    match line.char_indices().nth(MAX_FORM_ARGS_CHARS) {
        Some((at, _)) => format!("{}…", &line[..at]),
        None => line,
    }
}

/// `(added, removed)` lines a file-changing tool call made, or `None` for any other call. A
/// `write_file` cannot say what it overwrote, so it counts only the lines it wrote.
pub(crate) fn call_line_counts(name: &str, args: &Value) -> Option<(u32, u32)> {
    let name = name.strip_prefix("mcp__forge__").unwrap_or(name);
    let text = |key: &str| args.get(key).and_then(Value::as_str);
    match name {
        "write_file" | "append_file" | "create_file" => Some((line_count(text("content")?), 0)),
        "edit_file" if args.get("edits").is_some_and(Value::is_array) => {
            Some(anchored_lines(args.get("edits")?.as_array()?))
        }
        "edit_file" if args.get("old").is_some() || args.get("new").is_some() => {
            Some(edit_pair(args))
        }
        "multi_edit" => {
            let edits = args.get("edits")?.as_array()?;
            Some(
                edits
                    .iter()
                    .map(edit_pair)
                    .fold((0, 0), |a, b| (a.0 + b.0, a.1 + b.1)),
            )
        }
        "apply_patch" => Some(patch_lines(text("patch")?)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_question_form_keeps_every_option() {
        let args =
            json!({"questions": [{"question": "q".repeat(500), "options": [{"label": "last"}]}]});
        assert!(is_question_form("ask_user"));
        assert!(form_args(&args).contains("last"));
    }

    #[test]
    fn an_edit_counts_only_the_lines_that_differ() {
        let args = json!({"path": "a.rs", "old": "a\nb\nc\n", "new": "a\nB\nB2\nc\n"});
        assert_eq!(call_line_counts("edit_file", &args), Some((2, 1)));
    }

    #[test]
    fn a_write_counts_the_lines_it_wrote() {
        let args = json!({"path": "a.rs", "content": "x\ny\nz\n"});
        assert_eq!(call_line_counts("write_file", &args), Some((3, 0)));
        assert_eq!(
            call_line_counts("mcp__forge__write_file", &args),
            Some((3, 0))
        );
    }

    #[test]
    fn an_anchored_edit_counts_written_and_replaced_lines() {
        let args = json!({"path": "a.rs", "tag": "1A2B", "edits": [
            {"start": 3, "end": 5, "new": "x\ny\n"},
            {"after": 9, "new": "z"},
            {"start": 12, "new": ""}
        ]});
        assert_eq!(call_line_counts("edit_file", &args), Some((3, 4)));
    }

    #[test]
    fn a_multi_edit_sums_its_edits() {
        let args = json!({"edits": [
            {"path": "a", "old": "x", "new": "y"},
            {"path": "b", "old": "p\nq", "new": "p"},
        ]});
        assert_eq!(call_line_counts("multi_edit", &args), Some((1, 2)));
    }

    #[test]
    fn a_patch_counts_changed_lines_and_skips_file_headers() {
        let patch = "--- a/x\n+++ b/x\n@@ -1,2 +1,2 @@\n-old\n+new\n same\n+extra\n";
        assert_eq!(
            call_line_counts("apply_patch", &json!({ "patch": patch })),
            Some((2, 1))
        );
    }

    #[test]
    fn other_calls_have_no_line_counts() {
        assert_eq!(call_line_counts("shell", &json!({"command": "ls"})), None);
        assert_eq!(call_line_counts("edit_file", &json!({"path": "a"})), None);
    }
}
