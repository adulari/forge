//! `forge serve`'s GitHub Actions surface, over `gh workflow` and `gh run`.
//!
//! Routes (session-addressed; see [`crate::serve_pr`] for the `gh` runner and its failure model):
//! - `GET  /api/actions/workflows?session=`                 every workflow with its state
//! - `GET  /api/actions/runs?session=[&workflow=][&limit=]` recent runs
//! - `GET  /api/actions/run?session=&id=`                   one run with its jobs and steps
//! - `GET  /api/actions/inputs?session=&workflow=`          the `workflow_dispatch` inputs
//! - `POST /api/actions/dispatch` `{session, workflow, ref?, inputs?}`
//! - `POST /api/actions/{rerun,cancel,delete}` `{session, id[, failed_only]}`

use std::path::Path;
use std::sync::Arc;

use axum::extract::{Json, Query, State};
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::serve::{err_response, json_response, DaemonState};
use crate::serve_git::session_repo_root;
use crate::serve_pr::{fail, Gh, GhError};

pub(crate) fn routes(base: &str) -> Router<Arc<DaemonState>> {
    Router::new()
        .route(&format!("{base}/api/actions/workflows"), get(workflows))
        .route(&format!("{base}/api/actions/runs"), get(runs))
        .route(&format!("{base}/api/actions/run"), get(run_detail))
        .route(&format!("{base}/api/actions/inputs"), get(inputs))
        .route(&format!("{base}/api/actions/dispatch"), post(dispatch))
        .route(&format!("{base}/api/actions/rerun"), post(rerun))
        .route(&format!("{base}/api/actions/cancel"), post(cancel))
        .route(&format!("{base}/api/actions/delete"), post(delete))
}

#[derive(Deserialize)]
pub(crate) struct ReadQuery {
    session: String,
    #[serde(default)]
    workflow: Option<String>,
    #[serde(default)]
    id: Option<u64>,
    #[serde(default)]
    limit: Option<u32>,
}

async fn root_for(state: &DaemonState, session: &str) -> Result<std::path::PathBuf, Box<Response>> {
    session_repo_root(state, session)
        .await
        .map_err(|(status, message)| Box::new(err_response(status, &message)))
}

fn unavailable(error: &GhError) -> Option<Response> {
    matches!(error, GhError::Missing | GhError::SignedOut(_))
        .then(|| json_response(&json!({"available": false, "error": error.message(), "items": []})))
}

fn parse(raw: &str) -> Result<Value, GhError> {
    serde_json::from_str(raw).map_err(|e| GhError::Failed(format!("unreadable gh output: {e}")))
}

fn valid_workflow(workflow: &str) -> bool {
    !workflow.is_empty()
        && workflow.len() <= 200
        && !workflow.starts_with('-')
        && !workflow.chars().any(char::is_control)
}

fn valid_ref(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && !name.starts_with('-')
        && !name.contains("..")
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-' | '+' | '@'))
}

fn valid_input_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 100
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

async fn workflows(
    State(state): State<Arc<DaemonState>>,
    Query(query): Query<ReadQuery>,
) -> Response {
    let root = match root_for(&state, &query.session).await {
        Ok(root) => root,
        Err(response) => return *response,
    };
    match list_workflows(&Gh::system(), &root).await {
        Ok(items) => json_response(&json!({"available": true, "error": null, "items": items})),
        Err(e) => unavailable(&e).unwrap_or_else(|| fail(&e)),
    }
}

async fn list_workflows(gh: &Gh, root: &Path) -> Result<Value, GhError> {
    let raw = gh
        .run(
            root,
            &[
                "workflow",
                "list",
                "--all",
                "--limit",
                "100",
                "--json",
                "id,name,path,state",
            ],
            None,
        )
        .await?;
    parse(&raw)
}

async fn runs(State(state): State<Arc<DaemonState>>, Query(query): Query<ReadQuery>) -> Response {
    let root = match root_for(&state, &query.session).await {
        Ok(root) => root,
        Err(response) => return *response,
    };
    match list_runs(&Gh::system(), &root, query.workflow.as_deref(), query.limit).await {
        Ok(items) => json_response(&json!({"available": true, "error": null, "items": items})),
        Err(e) => unavailable(&e).unwrap_or_else(|| fail(&e)),
    }
}

async fn list_runs(
    gh: &Gh,
    root: &Path,
    workflow: Option<&str>,
    limit: Option<u32>,
) -> Result<Value, GhError> {
    let limit = limit.unwrap_or(25).clamp(1, 100).to_string();
    let mut args = vec![
        "run",
        "list",
        "--limit",
        &limit,
        "--json",
        "databaseId,displayTitle,workflowName,status,conclusion,headBranch,headSha,event,createdAt,updatedAt,url,number",
    ];
    if let Some(workflow) = workflow {
        if !valid_workflow(workflow) {
            return Err(GhError::Failed("invalid workflow".into()));
        }
        args.push("--workflow");
        args.push(workflow);
    }
    parse(&gh.run(root, &args, None).await?)
}

async fn run_detail(
    State(state): State<Arc<DaemonState>>,
    Query(query): Query<ReadQuery>,
) -> Response {
    let root = match root_for(&state, &query.session).await {
        Ok(root) => root,
        Err(response) => return *response,
    };
    let Some(id) = query.id else {
        return err_response(axum::http::StatusCode::BAD_REQUEST, "id is required");
    };
    let raw = Gh::system()
        .run(
            &root,
            &[
                "run",
                "view",
                &id.to_string(),
                "--json",
                "databaseId,displayTitle,workflowName,status,conclusion,headBranch,event,createdAt,updatedAt,url,jobs",
            ],
            None,
        )
        .await;
    match raw.and_then(|r| parse(&r)) {
        Ok(value) => json_response(&value),
        Err(e) => fail(&e),
    }
}

#[derive(Serialize, Debug, PartialEq, Eq)]
pub(crate) struct DispatchInput {
    name: String,
    description: String,
    required: bool,
    default: Option<String>,
    kind: String,
    options: Vec<String>,
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

fn unquote(value: &str) -> String {
    let value = value.trim();
    let value = match value.find(" #") {
        Some(at) if !value.starts_with(['"', '\'']) => &value[..at],
        _ => value,
    };
    let value = value.trim();
    for quote in ['"', '\''] {
        if value.len() >= 2 && value.starts_with(quote) && value.ends_with(quote) {
            return value[1..value.len() - 1].to_string();
        }
    }
    value.to_string()
}

/// `on.workflow_dispatch.inputs` from workflow YAML. A line scanner, not a YAML parser: it reads
/// the block form GitHub's own docs use (`inputs:` with one mapping per input, `options:` as a
/// list) and ignores anything fancier rather than guessing.
pub(crate) fn parse_dispatch_inputs(yaml: &str) -> Vec<DispatchInput> {
    let meaningful: Vec<&str> = yaml
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .collect();
    let Some(start) = meaningful
        .iter()
        .position(|l| l.trim_start().starts_with("workflow_dispatch:"))
    else {
        return Vec::new();
    };
    let dispatch_indent = indent_of(meaningful[start]);
    let Some(inputs_at) = meaningful[start + 1..]
        .iter()
        .take_while(|l| indent_of(l) > dispatch_indent)
        .position(|l| l.trim() == "inputs:")
        .map(|p| start + 1 + p)
    else {
        return Vec::new();
    };
    let inputs_indent = indent_of(meaningful[inputs_at]);
    let Some(name_indent) = meaningful
        .get(inputs_at + 1)
        .map(|l| indent_of(l))
        .filter(|n| *n > inputs_indent)
    else {
        return Vec::new();
    };
    let mut out: Vec<DispatchInput> = Vec::new();
    let mut in_options = false;
    for line in &meaningful[inputs_at + 1..] {
        let indent = indent_of(line);
        if indent <= inputs_indent {
            break;
        }
        let text = line.trim();
        if indent == name_indent {
            if let Some(name) = text.strip_suffix(':') {
                out.push(DispatchInput {
                    name: unquote(name),
                    description: String::new(),
                    required: false,
                    default: None,
                    kind: "string".into(),
                    options: Vec::new(),
                });
                in_options = false;
            }
            continue;
        }
        let Some(current) = out.last_mut() else {
            continue;
        };
        if in_options && text.starts_with('-') {
            current.options.push(unquote(text.trim_start_matches('-')));
            continue;
        }
        in_options = false;
        let Some((key, value)) = text.split_once(':') else {
            continue;
        };
        match key.trim() {
            "description" => current.description = unquote(value),
            "required" => current.required = unquote(value) == "true",
            "default" => current.default = Some(unquote(value)),
            "type" => current.kind = unquote(value),
            "options" => in_options = true,
            _ => {}
        }
    }
    out
}

async fn inputs(State(state): State<Arc<DaemonState>>, Query(query): Query<ReadQuery>) -> Response {
    let root = match root_for(&state, &query.session).await {
        Ok(root) => root,
        Err(response) => return *response,
    };
    let Some(workflow) = query.workflow.filter(|w| valid_workflow(w)) else {
        return err_response(axum::http::StatusCode::BAD_REQUEST, "workflow is required");
    };
    match Gh::system()
        .run(&root, &["workflow", "view", &workflow, "--yaml"], None)
        .await
    {
        Ok(yaml) => json_response(&json!({"inputs": parse_dispatch_inputs(&yaml)})),
        Err(e) => fail(&e),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DispatchRequest {
    session: String,
    workflow: String,
    #[serde(default, rename = "ref")]
    git_ref: Option<String>,
    #[serde(default)]
    inputs: Map<String, Value>,
}

async fn dispatch(
    State(state): State<Arc<DaemonState>>,
    Json(request): Json<DispatchRequest>,
) -> Response {
    let root = match root_for(&state, &request.session).await {
        Ok(root) => root,
        Err(response) => return *response,
    };
    match dispatch_in(&Gh::system(), &root, &request).await {
        Ok(()) => json_response(&json!({"ok": true})),
        Err(e) => fail(&e),
    }
}

async fn dispatch_in(gh: &Gh, root: &Path, request: &DispatchRequest) -> Result<(), GhError> {
    if !valid_workflow(&request.workflow) {
        return Err(GhError::Failed("invalid workflow".into()));
    }
    let mut args: Vec<String> = vec!["workflow".into(), "run".into(), request.workflow.clone()];
    if let Some(git_ref) = request.git_ref.as_deref().filter(|r| !r.is_empty()) {
        if !valid_ref(git_ref) {
            return Err(GhError::Failed("invalid ref".into()));
        }
        args.push("--ref".into());
        args.push(git_ref.into());
    }
    for (key, value) in &request.inputs {
        if !valid_input_key(key) {
            return Err(GhError::Failed(format!("invalid input name `{key}`")));
        }
        let value = match value {
            Value::String(s) => s.clone(),
            Value::Bool(b) => b.to_string(),
            Value::Number(n) => n.to_string(),
            _ => return Err(GhError::Failed(format!("input `{key}` must be a scalar"))),
        };
        args.push("-f".into());
        args.push(format!("{key}={value}"));
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    gh.run(root, &refs, None).await.map(|_| ())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RunRequest {
    session: String,
    id: u64,
    #[serde(default)]
    failed_only: bool,
}

async fn run_action(state: &DaemonState, request: RunRequest, verb: &str) -> Response {
    let root = match root_for(state, &request.session).await {
        Ok(root) => root,
        Err(response) => return *response,
    };
    match run_verb(&Gh::system(), &root, verb, request.id, request.failed_only).await {
        Ok(()) => json_response(&json!({"ok": true})),
        Err(e) => fail(&e),
    }
}

async fn run_verb(
    gh: &Gh,
    root: &Path,
    verb: &str,
    id: u64,
    failed_only: bool,
) -> Result<(), GhError> {
    let id = id.to_string();
    let mut args = vec!["run", verb, id.as_str()];
    if verb == "rerun" && failed_only {
        args.push("--failed");
    }
    gh.run(root, &args, None).await.map(|_| ())
}

async fn rerun(State(state): State<Arc<DaemonState>>, Json(request): Json<RunRequest>) -> Response {
    run_action(&state, request, "rerun").await
}

async fn cancel(
    State(state): State<Arc<DaemonState>>,
    Json(request): Json<RunRequest>,
) -> Response {
    run_action(&state, request, "cancel").await
}

async fn delete(
    State(state): State<Arc<DaemonState>>,
    Json(request): Json<RunRequest>,
) -> Response {
    run_action(&state, request, "delete").await
}

#[cfg(test)]
mod tests;
