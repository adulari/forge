//! The agent panel's compact timeline as plain data: the last reads, edits and commands, newest
//! first, with the file an edit touched. A port of the desktop `timelineModel.ts`.

use std::collections::{BTreeMap, VecDeque};

use crate::board::HistoryRow;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimelineKind {
    Prompt,
    Reply,
    Read,
    Edit,
    Command,
    Tool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelineItem {
    pub key: String,
    pub kind: TimelineKind,
    /// One short line: the target, the command, or the first line of the message.
    pub label: String,
    pub tool: Option<String>,
    pub failed: bool,
    pub adds: usize,
    pub dels: usize,
    /// The path an edit touched, as the model wrote it (often absolute).
    pub path: Option<String>,
    pub seq: i64,
}

const EDIT: &[&str] = &[
    "write",
    "edit",
    "patch",
    "apply",
    "create",
    "replace",
    "multiedit",
];
const READ: &[&str] = &[
    "read", "view", "cat", "ls", "list", "glob", "grep", "search", "find", "open",
];
const COMMAND: &[&str] = &[
    "shell", "bash", "exec", "run", "terminal", "command", "test",
];

fn has_prefix(tool: &str, words: &[&str]) -> bool {
    let lower = tool.to_ascii_lowercase();
    lower
        .split('_')
        .any(|segment| words.iter().any(|word| segment.starts_with(word)))
}

pub fn tool_kind(tool: Option<&str>) -> TimelineKind {
    match tool {
        None => TimelineKind::Tool,
        Some(tool) if has_prefix(tool, EDIT) => TimelineKind::Edit,
        Some(tool) if has_prefix(tool, COMMAND) => TimelineKind::Command,
        Some(tool) if has_prefix(tool, READ) => TimelineKind::Read,
        Some(_) => TimelineKind::Tool,
    }
}

const TARGET_KEYS: &[&str] = &[
    "path",
    "file",
    "filepath",
    "file_path",
    "command",
    "cmd",
    "script",
    "query",
    "pattern",
    "url",
    "prompt",
    "action",
];

fn parse_args(args: &str) -> Option<serde_json::Map<String, serde_json::Value>> {
    let text = args.trim();
    if !text.starts_with('{') {
        return None;
    }
    match serde_json::from_str::<serde_json::Value>(text).ok()? {
        serde_json::Value::Object(map) => Some(map),
        _ => None,
    }
}

fn first_line(text: &str, max: usize) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    if line.chars().count() > max {
        let cut: String = line.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}…")
    } else {
        line.to_string()
    }
}

fn target(args: &str, max: usize) -> String {
    let Some(parsed) = parse_args(args) else {
        return String::new();
    };
    for key in TARGET_KEYS {
        if let Some(value) = parsed.get(*key).and_then(|v| v.as_str()) {
            if !value.trim().is_empty() {
                return first_line(value, max);
            }
        }
    }
    String::new()
}

fn edit_path(args: &str) -> Option<String> {
    let parsed = parse_args(args)?;
    ["path", "file_path", "file", "filepath"]
        .iter()
        .find_map(|key| parsed.get(*key).and_then(|v| v.as_str()))
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// Whether a tool result reads as a failure.
pub fn result_failed(result: &str) -> bool {
    let head: String = result.trim_start().chars().take(200).collect();
    let lower = head.to_ascii_lowercase();
    if head.starts_with('✗') || lower.starts_with("error") || lower.starts_with("permission denied")
    {
        return true;
    }
    if let Some(rest) = head.strip_prefix("shell: ") {
        if rest.starts_with("timed out") || rest.starts_with("could not start") {
            return true;
        }
        if let Some(code) = rest.strip_prefix("exit ") {
            let code: String = code
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '-')
                .collect();
            return code != "0";
        }
    }
    false
}

/// `rows` are newest first, as `GET /api/history` serves them. Each call is paired first-in
/// first-out with the next result of the same tool, oldest to newest; the output is newest first.
pub fn build_timeline(rows: &[HistoryRow], limit: usize) -> Vec<TimelineItem> {
    let mut items: Vec<TimelineItem> = Vec::new();
    let mut awaiting: BTreeMap<String, VecDeque<usize>> = BTreeMap::new();
    for row in rows.iter().rev() {
        let is_tool = row.kind == "tool" || row.role == "tool";
        if !is_tool {
            let text = first_line(&row.content, 90);
            if text.is_empty() {
                continue;
            }
            let kind = if row.role == "user" && (row.kind.is_empty() || row.kind == "user") {
                TimelineKind::Prompt
            } else if row.role == "assistant" && row.kind != "system" {
                TimelineKind::Reply
            } else {
                continue;
            };
            items.push(TimelineItem {
                key: format!("m{}", row.seq),
                kind,
                label: text,
                tool: None,
                failed: false,
                adds: 0,
                dels: 0,
                path: None,
                seq: row.seq,
            });
            continue;
        }
        let name = row.tool.clone().unwrap_or_default();
        if row.tool_phase.as_deref() == Some("call") {
            let kind = tool_kind(row.tool.as_deref());
            let label = target(&row.content, 52);
            items.push(TimelineItem {
                key: format!("t{}", row.seq),
                kind,
                label: if label.is_empty() {
                    row.tool.clone().unwrap_or_else(|| "tool".to_string())
                } else {
                    label
                },
                tool: row.tool.clone(),
                failed: false,
                adds: row.adds.unwrap_or(0),
                dels: row.dels.unwrap_or(0),
                path: (kind == TimelineKind::Edit)
                    .then(|| edit_path(&row.content))
                    .flatten(),
                seq: row.seq,
            });
            awaiting.entry(name).or_default().push_back(items.len() - 1);
            continue;
        }
        if let Some(index) = awaiting.get_mut(&name).and_then(VecDeque::pop_front) {
            items[index].failed = result_failed(&row.content);
        }
    }
    items.reverse();
    items.truncate(limit);
    items
}

/// A model-written path (often absolute) -> the changed file it means, by whole path segments.
pub fn resolve_changed_path(written: &str, changed: &[String]) -> Option<String> {
    let normalized = written.strip_prefix("./").unwrap_or(written);
    if changed.iter().any(|c| c == normalized) {
        return Some(normalized.to_string());
    }
    changed
        .iter()
        .filter(|candidate| normalized.ends_with(&format!("/{candidate}")))
        .max_by_key(|candidate| candidate.len())
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(seq: i64, role: &str, kind: &str, content: &str) -> HistoryRow {
        HistoryRow {
            seq,
            role: role.to_string(),
            kind: kind.to_string(),
            content: content.to_string(),
            ..Default::default()
        }
    }

    fn tool(seq: i64, name: &str, phase: &str, content: &str) -> HistoryRow {
        HistoryRow {
            tool: Some(name.to_string()),
            tool_phase: Some(phase.to_string()),
            ..row(seq, "tool", "tool", content)
        }
    }

    #[test]
    fn classifies_tools_by_name_segment() {
        assert_eq!(tool_kind(Some("write_file")), TimelineKind::Edit);
        assert_eq!(tool_kind(Some("shell")), TimelineKind::Command);
        assert_eq!(tool_kind(Some("read_file")), TimelineKind::Read);
        assert_eq!(tool_kind(Some("spawn_agents")), TimelineKind::Tool);
        assert_eq!(tool_kind(None), TimelineKind::Tool);
    }

    #[test]
    fn pairs_a_call_with_its_result_and_lists_newest_first() {
        let rows = vec![
            tool(4, "shell", "result", "shell: exit 1\nboom"),
            tool(3, "shell", "call", r#"{"command":"cargo test"}"#),
            tool(2, "write_file", "result", "ok"),
            tool(
                1,
                "write_file",
                "call",
                r#"{"path":"/repo/src/a.rs","content":"x"}"#,
            ),
            row(0, "user", "user", "fix it"),
        ];
        let items = build_timeline(&rows, 10);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].kind, TimelineKind::Command);
        assert_eq!(items[0].label, "cargo test");
        assert!(items[0].failed);
        assert_eq!(items[1].kind, TimelineKind::Edit);
        assert!(!items[1].failed);
        assert_eq!(items[1].path.as_deref(), Some("/repo/src/a.rs"));
        assert_eq!(items[2].kind, TimelineKind::Prompt);
    }

    #[test]
    fn resolves_an_absolute_path_to_the_longest_changed_suffix() {
        let changed = vec!["a.rs".to_string(), "src/a.rs".to_string()];
        assert_eq!(
            resolve_changed_path("/repo/src/a.rs", &changed).as_deref(),
            Some("src/a.rs")
        );
        assert_eq!(
            resolve_changed_path("./a.rs", &changed).as_deref(),
            Some("a.rs")
        );
        assert!(resolve_changed_path("/repo/b.rs", &changed).is_none());
    }

    #[test]
    fn failure_reads_the_shell_exit_code() {
        assert!(result_failed("error: nope"));
        assert!(result_failed("shell: exit 2"));
        assert!(!result_failed("shell: exit 0\nfine"));
        assert!(!result_failed("done"));
    }
}
