//! `forge blame` — trace lines of AI-written code back to the session/model/turn that wrote
//! them, using the store's own tool-call records (no git dependency; docs/features/forge-blame.md).
//!
//! This module is pure over [`FileEditRow`]/current file content so the path-matching,
//! attribution, and rendering logic is unit-tested without a database; the CLI command resolves
//! the target file + queries the store and prints what these functions return.

use std::path::{Path, PathBuf};

use chrono::{Local, TimeZone};
use forge_store::{FileEditRow, TurnContext};

/// Marker `cap_result_json` (forge-store) appends to a truncated `args_json`. A contribution
/// carrying it is a PARTIAL write — the tail (and possibly the `content`/`new` field itself) may
/// be missing or cut mid-string — so it must never be attributed to a line.
const TRUNCATION_MARKER: &str = "…[truncated";

/// One line of the current file, enriched with the most recent recorded edit that wrote it (if
/// any). `model`/`session_id`/`seq`/`created_at` are all `None` together for an unattributed
/// ("human/unknown") line.
#[derive(Debug, Clone, PartialEq)]
pub struct LineAttribution {
    pub line: usize,
    pub text: String,
    pub model: Option<String>,
    pub session_id: Option<String>,
    pub seq: Option<i64>,
    pub created_at: Option<i64>,
}

impl LineAttribution {
    fn human(line: usize, text: &str) -> Self {
        Self {
            line,
            text: text.to_string(),
            model: None,
            session_id: None,
            seq: None,
            created_at: None,
        }
    }
}

/// A line stripped of leading/trailing whitespace that is too ambiguous to attribute even when it
/// happens to match a recorded edit verbatim — lone braces/parens/brackets (with an optional
/// trailing `,`/`;`) show up identically in nearly every edit, human or AI, so matching one would
/// be noise rather than signal. Blank lines are the same case (`chars().all` is vacuously true on
/// an empty string, so this single check covers both).
fn is_trivial_line(trimmed: &str) -> bool {
    trimmed.chars().all(|c| "{}()[],;".contains(c))
}

/// Resolve a tool call's raw `path` argument against the session's cwd (matching how the
/// `write_file`/`edit_file` tools themselves resolved it at call time — relative to the process
/// cwd, which is the session's cwd).
pub fn resolve_edit_path(session_cwd: &str, edit_path: &str) -> PathBuf {
    let p = Path::new(edit_path);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        Path::new(session_cwd).join(p)
    }
}

/// Filter `edits` down to the ones whose resolved path is the same file as `target`. Prefers an
/// exact match on the canonicalized form (handles `..`, symlinks, and relative-vs-absolute
/// spelling differences); falls back to a literal path-suffix match when either side can't be
/// canonicalized (e.g. `target` no longer exists on disk, or an edit's session directory has
/// since been removed).
pub fn matching_edits<'a>(target: &Path, edits: &'a [FileEditRow]) -> Vec<&'a FileEditRow> {
    let target_canon = target.canonicalize().ok();
    edits
        .iter()
        .filter(|e| {
            let resolved = resolve_edit_path(&e.session_cwd, &e.path);
            match (&target_canon, resolved.canonicalize().ok()) {
                (Some(t), Some(r)) => *t == r,
                _ => resolved == target || resolved.ends_with(target),
            }
        })
        .collect()
}

/// The text one recorded edit actually contributed to the file — `write_file`'s full `content`,
/// or `edit_file`'s `new` replacement snippet. `None` when the args were truncated at insert time
/// or fail to parse (an unparseable/partial edit is never attributed).
pub fn contributed_text(tool_name: &str, args_json: &str) -> Option<String> {
    if args_json.contains(TRUNCATION_MARKER) {
        return None;
    }
    let key = match tool_name {
        "write_file" => "content",
        "edit_file" => "new",
        _ => return None,
    };
    let v: serde_json::Value = serde_json::from_str(args_json).ok()?;
    v.get(key)?.as_str().map(str::to_string)
}

/// Attribute every line of `current_content` to the most recent (`created_at`) matching edit
/// among `edits`, or "(human/unknown)" when no edit's contributed text contains that line
/// verbatim (trimmed). Edits are processed oldest-first so a later edit's line always overrides
/// an earlier one that happened to write the same text — "latest edit wins".
pub fn attribute_lines(current_content: &str, edits: &[FileEditRow]) -> Vec<LineAttribution> {
    let mut ordered: Vec<&FileEditRow> = edits.iter().collect();
    ordered.sort_by_key(|e| e.created_at);

    let mut latest: std::collections::HashMap<String, &FileEditRow> =
        std::collections::HashMap::new();
    for &edit in &ordered {
        let Some(text) = contributed_text(&edit.tool_name, &edit.args_json) else {
            continue;
        };
        for line in text.lines() {
            let trimmed = line.trim();
            if is_trivial_line(trimmed) {
                continue;
            }
            latest.insert(trimmed.to_string(), edit);
        }
    }

    current_content
        .lines()
        .enumerate()
        .map(|(i, line)| {
            let line_no = i + 1;
            let trimmed = line.trim();
            if is_trivial_line(trimmed) {
                return LineAttribution::human(line_no, line);
            }
            match latest.get(trimmed) {
                Some(edit) => LineAttribution {
                    line: line_no,
                    text: line.to_string(),
                    model: edit.model.clone(),
                    session_id: Some(edit.session_id.clone()),
                    seq: Some(edit.seq),
                    created_at: Some(edit.created_at),
                },
                None => LineAttribution::human(line_no, line),
            }
        })
        .collect()
}

/// Short model label: strip the `provider::` prefix (mirrors `forge-tui`'s `model_short`), or
/// `"(human/unknown)"` when there's no model at all.
fn model_short(model: Option<&str>) -> String {
    match model {
        Some(m) if !m.is_empty() => m.split("::").last().unwrap_or(m).to_string(),
        _ => "(human/unknown)".to_string(),
    }
}

/// Truncate to `max` chars on a char boundary, appending an ellipsis when cut (mirrors
/// `replay::clip`).
fn clip(s: &str, max: usize) -> String {
    let one_line = s.replace('\n', " ");
    if one_line.chars().count() <= max {
        one_line
    } else {
        let head: String = one_line.chars().take(max).collect();
        format!("{head}…")
    }
}

fn fmt_time(epoch: i64) -> String {
    Local
        .timestamp_opt(epoch, 0)
        .single()
        .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| epoch.to_string())
}

/// Coarse "3d ago"-style relative age. `now` is passed in (rather than read internally) so this
/// stays pure/deterministic and unit-testable.
fn relative_age(now: i64, then: i64) -> String {
    let secs = (now - then).max(0);
    if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 3_600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3_600)
    } else {
        format!("{}d ago", secs / 86_400)
    }
}

/// Whole-file summary: one aligned row per line — line number, code (truncated), model, session
/// id prefix, and relative age.
pub fn render_blame(attributions: &[LineAttribution], now: i64) -> String {
    let mut out = String::new();
    for a in attributions {
        let model = model_short(a.model.as_deref());
        let sess8: String = a
            .session_id
            .as_deref()
            .unwrap_or("")
            .chars()
            .take(8)
            .collect();
        let age = a
            .created_at
            .map(|ts| relative_age(now, ts))
            .unwrap_or_default();
        out.push_str(&format!(
            "{:>5}  {:<60}  {:<16}  {:<8}  {}\n",
            a.line,
            clip(&a.text, 60),
            model,
            sess8,
            age,
        ));
    }
    out
}

/// Full provenance card for one line: model, session, timestamp, the user prompt that started
/// the turn, the assistant's own message content for that turn, and a `forge replay` pointer.
/// For an unattributed line, says so and stops there — there's no session to point at.
pub fn render_why(attribution: &LineAttribution, turn: &TurnContext, now: i64) -> String {
    let mut out = String::new();
    out.push_str(&format!("line {}\n", attribution.line));
    out.push_str(&format!("  text      {}\n", attribution.text.trim()));
    let Some(session_id) = &attribution.session_id else {
        out.push_str("  (human/unknown) — no AI-authored edit recorded for this line\n");
        return out;
    };
    let sess8: String = session_id.chars().take(8).collect();
    out.push_str(&format!(
        "  model     {}\n",
        model_short(attribution.model.as_deref())
    ));
    out.push_str(&format!("  session   {sess8}\n"));
    if let Some(seq) = attribution.seq {
        out.push_str(&format!("  turn      seq {seq}\n"));
    }
    if let Some(ts) = attribution.created_at {
        out.push_str(&format!(
            "  when      {} ({})\n",
            fmt_time(ts),
            relative_age(now, ts)
        ));
    }
    if let Some(p) = &turn.user_prompt {
        if !p.trim().is_empty() {
            out.push_str(&format!("  prompt    {}\n", clip(p, 200)));
        }
    }
    if let Some(a) = &turn.assistant_content {
        if !a.trim().is_empty() {
            out.push_str(&format!("  assistant {}\n", clip(a, 200)));
        }
    }
    out.push_str(&format!("\n  → forge replay {sess8}\n"));
    out
}

/// `--json`: a JSON array of `{line, text, model, session, seq, created_at}`, one per line.
pub fn render_json(attributions: &[LineAttribution]) -> String {
    let arr: Vec<serde_json::Value> = attributions
        .iter()
        .map(|a| {
            serde_json::json!({
                "line": a.line,
                "text": a.text,
                "model": a.model,
                "session": a.session_id,
                "seq": a.seq,
                "created_at": a.created_at,
            })
        })
        .collect();
    serde_json::to_string_pretty(&arr).unwrap_or_else(|e| format!("[{{\"error\":\"{e}\"}}]"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::too_many_arguments)]
    fn edit(
        tool_name: &str,
        args_json: &str,
        session_id: &str,
        cwd: &str,
        path: &str,
        model: Option<&str>,
        seq: i64,
        created_at: i64,
    ) -> FileEditRow {
        FileEditRow {
            tool_name: tool_name.to_string(),
            args_json: args_json.to_string(),
            path: path.to_string(),
            session_id: session_id.to_string(),
            session_cwd: cwd.to_string(),
            model: model.map(String::from),
            seq,
            created_at,
        }
    }

    #[test]
    fn attribute_lines_picks_the_latest_edit_when_two_write_the_same_line() {
        let e1 = edit(
            "write_file",
            r#"{"path":"a.rs","content":"fn old() {}\nlet shared = 1;\n"}"#,
            "sess-one",
            "/repo",
            "a.rs",
            Some("openai::gpt-4o"),
            1,
            1_000,
        );
        let e2 = edit(
            "edit_file",
            r#"{"path":"a.rs","old":"x","new":"let shared = 1;\n"}"#,
            "sess-two",
            "/repo",
            "a.rs",
            Some("anthropic::claude"),
            5,
            2_000,
        );
        let current = "let shared = 1;\n";
        let attrs = attribute_lines(current, &[e1, e2]);
        assert_eq!(attrs.len(), 1);
        assert_eq!(attrs[0].session_id.as_deref(), Some("sess-two"));
        assert_eq!(attrs[0].model.as_deref(), Some("anthropic::claude"));
    }

    #[test]
    fn attribute_lines_marks_unmatched_lines_as_human_unknown() {
        let e1 = edit(
            "write_file",
            r#"{"path":"a.rs","content":"fn known() {}\n"}"#,
            "sess-one",
            "/repo",
            "a.rs",
            Some("m"),
            1,
            1_000,
        );
        let current = "fn known() {}\nfn handwritten() {}\n";
        let attrs = attribute_lines(current, &[e1]);
        assert!(attrs[0].session_id.is_some());
        assert!(attrs[1].session_id.is_none());
        assert_eq!(attrs[1].text, "fn handwritten() {}");
    }

    #[test]
    fn attribute_lines_skips_contributions_carrying_the_truncation_marker() {
        let truncated = edit(
            "write_file",
            r#"{"path":"a.rs","content":"let x = 1;…[truncated 900000 bytes]"#,
            "sess-one",
            "/repo",
            "a.rs",
            Some("m"),
            1,
            1_000,
        );
        let current = "let x = 1;\n";
        let attrs = attribute_lines(current, &[truncated]);
        assert!(attrs[0].session_id.is_none());
    }

    #[test]
    fn attribute_lines_never_attributes_blank_or_trivial_lines() {
        let e1 = edit(
            "write_file",
            r#"{"path":"a.rs","content":"fn f() {\n\n}\n"}"#,
            "sess-one",
            "/repo",
            "a.rs",
            Some("m"),
            1,
            1_000,
        );
        let current = "fn f() {\n\n}\n";
        let attrs = attribute_lines(current, &[e1]);
        // "fn f() {" is real content and matches → attributed.
        assert!(attrs[0].session_id.is_some());
        // blank line and lone "}" are trivial → always human/unknown even though they
        // technically appear in the edit's contributed text too.
        assert!(attrs[1].session_id.is_none());
        assert!(attrs[2].session_id.is_none());
    }

    #[test]
    fn resolve_edit_path_joins_relative_paths_against_session_cwd() {
        assert_eq!(
            resolve_edit_path("/repo", "src/main.rs"),
            PathBuf::from("/repo/src/main.rs")
        );
    }

    #[test]
    fn resolve_edit_path_leaves_absolute_paths_untouched() {
        assert_eq!(
            resolve_edit_path("/repo", "/etc/hosts"),
            PathBuf::from("/etc/hosts")
        );
    }

    #[test]
    fn matching_edits_falls_back_to_suffix_match_when_canonicalize_fails() {
        // Neither the target nor the edit's resolved path exists on disk, so canonicalize()
        // fails for both — matching_edits must still line them up on their literal path.
        let e1 = edit(
            "write_file",
            r#"{"path":"src/gone.rs","content":"x"}"#,
            "sess-one",
            "/nonexistent-repo-xyz",
            "src/gone.rs",
            Some("m"),
            1,
            1_000,
        );
        let e2 = edit(
            "write_file",
            r#"{"path":"src/other.rs","content":"y"}"#,
            "sess-one",
            "/nonexistent-repo-xyz",
            "src/other.rs",
            Some("m"),
            2,
            2_000,
        );
        let target = Path::new("/nonexistent-repo-xyz/src/gone.rs");
        let edits = [e1, e2];
        let matches = matching_edits(target, &edits);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].path, "src/gone.rs");
    }

    #[test]
    fn render_blame_lists_one_row_per_line() {
        let attrs = vec![
            LineAttribution {
                line: 1,
                text: "let x = 1;".to_string(),
                model: Some("anthropic::claude-opus-4-8".to_string()),
                session_id: Some("abcdef1234567890".to_string()),
                seq: Some(3),
                created_at: Some(1_000),
            },
            LineAttribution::human(2, "// handwritten"),
        ];
        let out = render_blame(&attrs, 1_000 + 3_600);
        assert!(out.contains("claude-opus-4-8"));
        assert!(out.contains("abcdef12"));
        assert!(out.contains("1h ago"));
        assert!(out.contains("(human/unknown)"));
    }

    #[test]
    fn render_why_reports_human_unknown_without_a_session_pointer() {
        let attr = LineAttribution::human(2, "// handwritten");
        let out = render_why(&attr, &TurnContext::default(), 2_000);
        assert!(out.contains("human/unknown"));
        assert!(!out.contains("forge replay"));
    }

    #[test]
    fn render_why_includes_prompt_and_replay_pointer_when_attributed() {
        let attr = LineAttribution {
            line: 5,
            text: "let shared = 1;".to_string(),
            model: Some("anthropic::claude".to_string()),
            session_id: Some("abcdef1234567890".to_string()),
            seq: Some(7),
            created_at: Some(1_000),
        };
        let turn = TurnContext {
            user_prompt: Some("add the shared counter".to_string()),
            assistant_content: Some("Added it.".to_string()),
        };
        let out = render_why(&attr, &turn, 1_000);
        assert!(out.contains("add the shared counter"));
        assert!(out.contains("forge replay abcdef12"));
    }

    #[test]
    fn render_json_is_valid_json_with_expected_fields() {
        let attrs = vec![LineAttribution {
            line: 1,
            text: "x".to_string(),
            model: Some("m".to_string()),
            session_id: Some("s".to_string()),
            seq: Some(2),
            created_at: Some(3),
        }];
        let out = render_json(&attrs);
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid JSON");
        assert_eq!(v[0]["line"], 1);
        assert_eq!(v[0]["model"], "m");
        assert_eq!(v[0]["session"], "s");
    }
}
