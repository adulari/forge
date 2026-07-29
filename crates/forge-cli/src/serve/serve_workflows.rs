//! Saved-workflow library projection for the Serve API.
//!
//! This owner parses authored workflow metadata and joins scripts to the durable run history for
//! the selected workspace. Keeping that policy together prevents the HTTP composition root from
//! owning a second workflow catalog implementation.

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::response::Response;

use super::{err_response, json_response, DaemonState};

#[derive(serde::Deserialize)]
pub(super) struct WorkflowsParams {
    #[serde(default)]
    session: String,
}

#[derive(serde::Serialize)]
struct WorkflowRow {
    name: String,
    description: String,
    when_to_use: Option<String>,
    phases: Vec<String>,
    args: Vec<WorkflowArg>,
    runs: Vec<WorkflowRun>,
}

const WORKFLOW_RUNS_PER_ROW: usize = 10;

#[derive(serde::Serialize)]
struct WorkflowArg {
    name: String,
    arg_type: Option<String>,
    required: bool,
    description: Option<String>,
    default: Option<String>,
}

#[derive(serde::Serialize)]
struct WorkflowRun {
    started_at: i64,
    finished_at: Option<i64>,
    ok: Option<bool>,
    summary: Option<String>,
    status: String,
    session_id: String,
    phases: i64,
    agents: i64,
    cost_usd: f64,
}

impl From<forge_store::WorkflowRun> for WorkflowRun {
    fn from(run: forge_store::WorkflowRun) -> Self {
        Self {
            started_at: run.started_at,
            finished_at: run.finished_at,
            ok: match run.status.as_str() {
                "ok" => Some(true),
                "failed" => Some(false),
                _ => None,
            },
            summary: run.summary,
            status: run.status,
            session_id: run.session_id,
            phases: run.phases,
            agents: run.agents,
            cost_usd: run.cost_usd,
        }
    }
}

fn meta_arg_objects(meta: &str) -> Vec<&str> {
    let Some(idx) = meta.find("args:") else {
        return Vec::new();
    };
    let tail = &meta[idx + "args:".len()..];
    let Some(open_bracket) = tail.find('[') else {
        return Vec::new();
    };
    let body = &tail[open_bracket + 1..];
    let mut objects = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    let mut in_str: Option<char> = None;
    let mut prev_backslash = false;
    for (i, c) in body.char_indices() {
        if let Some(quote) = in_str {
            if prev_backslash {
                prev_backslash = false;
            } else if c == '\\' {
                prev_backslash = true;
            } else if c == quote {
                in_str = None;
            }
            continue;
        }
        match c {
            '\'' | '"' | '`' => in_str = Some(c),
            '{' => {
                if depth == 0 {
                    start = i;
                }
                depth += 1;
            }
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    objects.push(&body[start..=i]);
                }
            }
            ']' if depth == 0 => break,
            _ => {}
        }
    }
    objects
}

fn meta_args(meta: &str) -> Vec<WorkflowArg> {
    meta_arg_objects(meta)
        .into_iter()
        .filter_map(|object| {
            let name = meta_string_field(object, "name")?;
            Some(WorkflowArg {
                name,
                arg_type: meta_string_field(object, "type"),
                required: object.contains("required: true")
                    || object.contains("required:true")
                    || object.contains("required: !0"),
                description: meta_string_field(object, "description"),
                default: meta_string_field(object, "default"),
            })
        })
        .collect()
}

fn meta_string_field(meta: &str, field: &str) -> Option<String> {
    meta_string_field_at(meta, field).map(|(value, _)| value)
}

fn meta_string_field_at(meta: &str, field: &str) -> Option<(String, usize)> {
    let idx = meta.find(&format!("{field}:"))?;
    let field_end = idx + field.len() + 1;
    let rest = &meta[field_end..];
    let whitespace = rest.len() - rest.trim_start().len();
    let rest = &rest[whitespace..];
    let quote = rest.chars().next()?;
    if quote != '\'' && quote != '"' && quote != '`' {
        return None;
    }
    let body = &rest[1..];
    let mut out = String::new();
    let mut chars = body.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(escaped) = chars.next() {
                out.push(escaped);
            }
        } else if c == quote {
            return Some((
                out,
                field_end + whitespace + 1 + body.len() - chars.as_str().len(),
            ));
        } else {
            out.push(c);
        }
    }
    None
}

fn meta_literal(script: &str) -> Option<&str> {
    let start = script.find("export const meta")?;
    let open = script[start..].find('{')? + start;
    let mut depth = 0usize;
    let mut in_str: Option<char> = None;
    let mut prev_backslash = false;
    for (i, c) in script[open..].char_indices() {
        if let Some(q) = in_str {
            if prev_backslash {
                prev_backslash = false;
            } else if c == '\\' {
                prev_backslash = true;
            } else if c == q {
                in_str = None;
            }
            continue;
        }
        match c {
            '\'' | '"' | '`' => in_str = Some(c),
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&script[open..=open + i]);
                }
            }
            _ => {}
        }
    }
    None
}

fn meta_phase_titles(meta: &str) -> Vec<String> {
    let Some(idx) = meta.find("phases:") else {
        return Vec::new();
    };
    let Some(tail) = meta.get(idx..) else {
        return Vec::new();
    };
    let Some(end) = tail.find(']') else {
        return Vec::new();
    };
    let mut titles = Vec::new();
    let mut rest = &tail[..end];
    while let Some((title, consumed)) = meta_string_field_at(rest, "title") {
        titles.push(title);
        rest = rest.get(consumed..).unwrap_or("");
    }
    titles
}

pub(super) async fn workflows_page(
    State(state): State<Arc<DaemonState>>,
    Query(params): Query<WorkflowsParams>,
) -> Response {
    let cwd = if params.session.is_empty() {
        state.default_cwd.clone()
    } else {
        match state.registry.get(&params.session).await {
            Some(handle) => handle.cwd.clone(),
            None => state.default_cwd.clone(),
        }
    };
    let store = state.store.clone();
    let rows = tokio::task::spawn_blocking(move || {
        let dir = std::path::Path::new(&cwd).join(".forge").join("workflows");
        let run_key = std::fs::canonicalize(&cwd)
            .map(|path| path.display().to_string())
            .unwrap_or_else(|_| cwd.clone());
        let mut rows: Vec<WorkflowRow> = Vec::new();
        let Ok(entries) = std::fs::read_dir(dir) else {
            return rows;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("js") {
                continue;
            }
            let Some(name) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            let script = std::fs::read_to_string(&path).unwrap_or_default();
            let meta = meta_literal(&script).unwrap_or("");
            rows.push(WorkflowRow {
                name: name.to_string(),
                description: meta_string_field(meta, "description").unwrap_or_default(),
                when_to_use: meta_string_field(meta, "whenToUse"),
                phases: meta_phase_titles(meta),
                args: meta_args(meta),
                runs: store
                    .list_workflow_runs(name, &run_key, WORKFLOW_RUNS_PER_ROW)
                    .unwrap_or_default()
                    .into_iter()
                    .map(WorkflowRun::from)
                    .collect(),
            });
        }
        rows.sort_by(|left, right| left.name.cmp(&right.name));
        rows
    })
    .await;
    match rows {
        Ok(rows) => json_response(&rows),
        Err(_) => err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "could not read workflows",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{meta_args, meta_literal, meta_phase_titles, meta_string_field, WorkflowRun};

    #[test]
    fn metadata_parser_preserves_authored_library_fields() {
        let script = r#"
export const meta = {
  name: 'code-review',
  description: 'Review a diff { and } braces',
  whenToUse: 'When you\'re unsure it is right',
  phases: [
    { title: 'Scan', prompt: 'p1' },
    { title: 'Verify', prompt: 'p2' },
    { title: 'Report', prompt: 'p3' },
  ],
};
export async function run() {}
"#;
        let meta = meta_literal(script).expect("meta literal");
        assert!(!meta.contains("export async function"));
        assert_eq!(
            meta_string_field(meta, "description").as_deref(),
            Some("Review a diff { and } braces")
        );
        assert_eq!(
            meta_string_field(meta, "whenToUse").as_deref(),
            Some("When you're unsure it is right")
        );
        assert_eq!(meta_phase_titles(meta), ["Scan", "Verify", "Report"]);
        assert!(meta_literal("no meta here").is_none());
    }

    #[test]
    fn phase_scanner_advances_over_escaped_titles_and_spacing() {
        let meta = r#"{ phases: [{ title: 'Don\'t skip' }, { title:   "Verify" }] }"#;
        assert_eq!(meta_phase_titles(meta), ["Don't skip", "Verify"]);
    }

    #[test]
    fn metadata_args_require_an_authored_name() {
        let meta = "{ name: 'x', args: [{ name: 'target', type: 'path', required: true, description: 'what to scan' }, { type: 'string' }] }";
        let args = meta_args(meta);
        assert_eq!(args.len(), 1);
        assert_eq!(args[0].name, "target");
        assert_eq!(args[0].arg_type.as_deref(), Some("path"));
        assert!(args[0].required);
        assert_eq!(args[0].description.as_deref(), Some("what to scan"));
        assert!(meta_args("{ name: 'x' }").is_empty());
    }

    #[test]
    fn interrupted_runs_do_not_claim_a_boolean_verdict() {
        let row = |status: &str| {
            WorkflowRun::from(forge_store::WorkflowRun {
                id: "r".into(),
                name: "audit".into(),
                session_id: "s".into(),
                cwd: "/repo".into(),
                started_at: 1_770_000_000,
                finished_at: None,
                status: status.into(),
                summary: None,
                phases: 0,
                agents: 0,
                cost_usd: 0.0,
            })
        };
        assert_eq!(row("ok").ok, Some(true));
        assert_eq!(row("failed").ok, Some(false));
        assert_eq!(row("interrupted").ok, None);
        assert_eq!(row("running").ok, None);
    }
}
