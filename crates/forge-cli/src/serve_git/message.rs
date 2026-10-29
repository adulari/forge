//! `POST /api/git/commit-message` — a commit message for what is staged, written by a trivial-tier
//! model call (the tier the shell-error interceptor uses) over the staged diff.
//!
//! Nothing is committed or staged here; the dock puts the text in its message well and the person
//! edits or discards it. The diff the model sees is bounded (lockfiles and generated noise are
//! summarised, each file is capped, the whole prompt is capped), so a huge staged change costs the
//! same as a small one.

use std::time::Duration;

use forge_provider::StreamEvent;
use forge_types::{Message, TaskTier};

use super::*;

const PATCH_BUDGET_CHARS: usize = 24_000;
const FILE_BUDGET_CHARS: usize = 4_000;
const CALL_TIMEOUT: Duration = Duration::from_secs(45);
const MAX_CANDIDATES: usize = 3;
const MAX_LANGUAGE_CHARS: usize = 40;

const SYSTEM: &str = "You write git commit messages. Reply with the commit message only: no \
preamble, no code fences, no quotes. Format: a conventional-commit subject line \
(type(scope): summary, imperative mood, at most 72 characters, no trailing period), then if the \
change needs it a blank line and a short body of at most a few lines explaining what changed and \
why. Base it strictly on the diff; do not invent changes. The type prefix (feat, fix, refactor, \
docs, test, chore, perf, build, ci) stays in English.";

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GitCommitMessageRequest {
    session: String,
    /// Overrides `[git] commit_language` for this one message.
    #[serde(default)]
    language: Option<String>,
}

#[derive(serde::Serialize)]
pub(crate) struct GitCommitMessageResponse {
    message: String,
    model: String,
    language: String,
}

/// `git diff --cached` condensed to what a model needs to name the change.
pub(super) struct StagedChange {
    pub(super) stat: String,
    pub(super) patch: String,
    pub(super) recent: Vec<String>,
}

const NOISY_SUFFIXES: [&str; 7] = [
    ".lock",
    "package-lock.json",
    "pnpm-lock.yaml",
    "yarn.lock",
    "go.sum",
    ".min.js",
    ".map",
];

fn is_noisy(path: &str) -> bool {
    NOISY_SUFFIXES.iter().any(|suffix| path.ends_with(suffix))
}

fn clip(text: &str, limit: usize) -> &str {
    if text.len() <= limit {
        return text;
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Keep each file's diff under [`FILE_BUDGET_CHARS`] and the whole under [`PATCH_BUDGET_CHARS`];
/// lockfiles and sourcemaps are named but not shown.
pub(super) fn condense_patch(raw: &str) -> String {
    let mut out = String::new();
    for section in raw.split("\ndiff --git ") {
        let section = section.trim_start_matches("diff --git ");
        if section.trim().is_empty() {
            continue;
        }
        let header = section.lines().next().unwrap_or_default();
        let path = header.rsplit(" b/").next().unwrap_or_default();
        out.push_str("diff --git ");
        if is_noisy(path) {
            out.push_str(header);
            out.push_str("\n(generated file changed; contents omitted)\n");
        } else {
            let clipped = clip(section, FILE_BUDGET_CHARS);
            out.push_str(clipped);
            if clipped.len() < section.len() {
                out.push_str("\n[diff truncated]");
            }
            if !out.ends_with('\n') {
                out.push('\n');
            }
        }
        if out.len() >= PATCH_BUDGET_CHARS {
            out.push_str("[remaining files omitted]\n");
            break;
        }
    }
    out
}

pub(super) fn staged_change(root: &Path) -> Result<StagedChange, String> {
    let names = git_stdout(root, &["diff", "--cached", "--name-only"]).unwrap_or_default();
    if names.is_empty() {
        return Err("nothing staged to commit".to_string());
    }
    let stat = String::from_utf8_lossy(&git_raw(
        root,
        &[
            "diff",
            "--cached",
            "--no-color",
            "--stat=100",
            "--find-renames",
        ],
    )?)
    .into_owned();
    let raw = String::from_utf8_lossy(&git_raw(
        root,
        &[
            "-c",
            "core.quotePath=false",
            "diff",
            "--cached",
            "--no-color",
            "--unified=2",
            "--find-renames",
        ],
    )?)
    .into_owned();
    let recent = git_stdout(root, &["log", "-5", "--format=%s"])
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect();
    Ok(StagedChange {
        stat: stat.trim().to_string(),
        patch: condense_patch(&raw),
        recent,
    })
}

/// A language name the model can be told about, or `None` when the request's value is unusable
/// (empty, absurdly long, or carrying line breaks that could rewrite the instruction).
fn sanitize_language(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let usable = !trimmed.is_empty()
        && trimmed.chars().count() <= MAX_LANGUAGE_CHARS
        && !trimmed.chars().any(char::is_control);
    usable.then(|| trimmed.to_string())
}

pub(super) fn build_prompt(change: &StagedChange, language: &str) -> String {
    let mut prompt = format!("Write the commit message in {language}.\n\n");
    if !change.recent.is_empty() {
        prompt.push_str("Recent commit subjects in this repository, for style:\n");
        for subject in &change.recent {
            prompt.push_str("- ");
            prompt.push_str(subject);
            prompt.push('\n');
        }
        prompt.push('\n');
    }
    prompt.push_str("Staged change summary:\n");
    prompt.push_str(&change.stat);
    prompt.push_str("\n\nStaged diff:\n");
    prompt.push_str(&change.patch);
    prompt
}

/// Strip the wrapping a chatty model adds despite the instruction: a code fence, surrounding
/// quotes, a leading "Commit message:" label.
pub(super) fn clean_message(raw: &str) -> String {
    let mut text = raw.trim();
    if let Some(rest) = text.strip_prefix("```") {
        let rest = rest.split_once('\n').map(|(_, body)| body).unwrap_or(rest);
        text = rest.trim_end().trim_end_matches("```").trim();
    }
    for label in ["Commit message:", "commit message:", "Commit Message:"] {
        if let Some(rest) = text.strip_prefix(label) {
            text = rest.trim();
        }
    }
    let unquoted = text
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or(text);
    unquoted.trim().to_string()
}

/// A subject line for the offline `--mock` daemon, which has no model to ask.
fn offline_message(change: &StagedChange) -> String {
    let files: Vec<&str> = change
        .stat
        .lines()
        .filter_map(|line| line.split('|').next())
        .map(str::trim)
        .filter(|name| !name.is_empty() && !name.contains("changed"))
        .collect();
    match files.as_slice() {
        [] => "chore: update files".to_string(),
        [one] => format!("chore: update {one}"),
        [first, rest @ ..] => format!("chore: update {first} and {} more", rest.len()),
    }
}

/// Pick the models to try for a one-shot trivial-tier call: the router's choice, then its
/// fallbacks, API models before CLI bridges (a bridge spawns a whole agent process for a
/// ten-word answer, so it is only used when nothing else is left).
async fn candidate_models(
    config: &forge_config::Config,
    router: &Arc<dyn forge_mesh::Router>,
    store: &forge_store::Store,
    prompt: &str,
) -> Vec<String> {
    let budget = forge_mesh::BudgetState {
        spent_today_usd: store.spend_today_usd().unwrap_or(0.0),
        daily_cap_usd: config.mesh.daily_budget_usd,
        spent_week_usd: store.spend_this_week_usd().unwrap_or(0.0),
        weekly_cap_usd: config.mesh.weekly_budget_usd,
        spent_month_usd: store.spend_this_month_usd().unwrap_or(0.0),
        monthly_cap_usd: config.mesh.monthly_cap_usd,
        warn_fraction: config.mesh.warn_threshold,
        min_context_tokens: None,
    };
    let readiness = forge_core::readiness::ProviderReadiness::snapshot(config, store);
    let decision = router
        .route_hinted(
            prompt,
            false,
            budget,
            &readiness.health,
            &readiness.quota,
            Some(TaskTier::Trivial),
            None,
            &forge_types::ProjectContext::default(),
        )
        .await;
    if decision.unroutable {
        return Vec::new();
    }
    let mut models = vec![decision.model];
    models.extend(decision.fallbacks);
    let (mut api, bridged): (Vec<String>, Vec<String>) = models
        .into_iter()
        .partition(|model| !forge_provider::is_cli_bridge(model));
    api.extend(bridged);
    api.truncate(MAX_CANDIDATES);
    api
}

async fn generate(
    state: &DaemonState,
    session: &str,
    root: &Path,
    request_language: Option<&str>,
) -> Result<GitCommitMessageResponse, String> {
    let root_owned = root.to_path_buf();
    let change = tokio::task::spawn_blocking(move || staged_change(&root_owned))
        .await
        .map_err(|_| "git failed".to_string())??;
    let config = forge_config::load().map_err(|e| e.to_string())?;
    let language = request_language
        .and_then(sanitize_language)
        .or_else(|| forge_config::commit_language::repo_commit_language(root))
        .unwrap_or_else(|| {
            forge_config::commit_language::commit_language_or_default(&config.git.commit_language)
                .to_string()
        });
    if state.mock {
        return Ok(GitCommitMessageResponse {
            message: offline_message(&change),
            model: "mock".to_string(),
            language,
        });
    }
    let prompt = build_prompt(&change, &language);
    let (provider, router) = crate::cli::commands::models::build_provider_and_router(
        &config,
        false,
        None,
        None,
        Default::default(),
        Default::default(),
    );
    let models = candidate_models(&config, &router, &state.store, "write a commit message").await;
    if models.is_empty() {
        return Err("no model is available for a commit message right now".to_string());
    }
    let messages = [Message::system(SYSTEM), Message::user(prompt)];
    let mut last = String::new();
    for model in models {
        let mut sink = |_: StreamEvent| {};
        let call = provider.complete(&model, &messages, &[], &mut sink);
        match tokio::time::timeout(CALL_TIMEOUT, call).await {
            Ok(Ok(response)) => {
                let _ = state.store.record_side_call_usage_for(
                    session,
                    "commit-message",
                    Some(&model),
                    &response.usage,
                );
                let message = clean_message(&response.content);
                if message.is_empty() {
                    last = format!("{model} returned an empty message");
                    continue;
                }
                return Ok(GitCommitMessageResponse {
                    message,
                    model,
                    language,
                });
            }
            Ok(Err(error)) => last = format!("{model}: {error}"),
            Err(_) => last = format!("{model} did not answer within {}s", CALL_TIMEOUT.as_secs()),
        }
    }
    Err(format!("could not generate a commit message ({last})"))
}

/// `POST /api/git/commit-message` `{session, language?}` → `{message, model, language}`.
pub(crate) async fn git_commit_message(
    State(state): State<Arc<DaemonState>>,
    Json(request): Json<GitCommitMessageRequest>,
) -> Response {
    let Some(dir) = session_dir(&state, &request.session).await else {
        return no_such_session();
    };
    let root = match tokio::task::spawn_blocking(move || repo_root_of(&dir)).await {
        Ok(Ok(root)) => root,
        Ok(Err(message)) => return err_response(axum::http::StatusCode::BAD_REQUEST, &message),
        Err(_) => return err_response(axum::http::StatusCode::INTERNAL_SERVER_ERROR, "git failed"),
    };
    match generate(&state, &request.session, &root, request.language.as_deref()).await {
        Ok(response) => json_response(&response),
        Err(message) if message == "nothing staged to commit" => {
            err_response(axum::http::StatusCode::BAD_REQUEST, &message)
        }
        Err(message) => err_response(axum::http::StatusCode::BAD_GATEWAY, &message),
    }
}

#[cfg(test)]
mod tests;
