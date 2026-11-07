//! Derive a local "CI plan" from `.github/workflows/*.yml`: the `run:` steps of jobs that fire on
//! push / pull_request on a Linux runner, with workflow / job / step `env:` applied. Everything
//! that cannot run on this machine is listed as a skip with its reason, never dropped silently.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::yaml::{self, Yaml};

/// Steps that run on this machine as they would in CI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CiStep {
    /// `workflow::job::name` — what `.forge/ci.toml` selectors match against.
    pub key: String,
    pub workflow: String,
    pub job: String,
    pub name: String,
    pub run: String,
    pub env: BTreeMap<String, String>,
    pub cwd: Option<String>,
    pub shell: Option<String>,
    pub timeout_secs: Option<u64>,
    pub allow_failure: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CiSkip {
    pub workflow: String,
    pub job: Option<String>,
    pub step: Option<String>,
    pub reason: String,
    /// The step as it would run, when only its environment stopped it — `include` in
    /// `.forge/ci.toml` can pull it back in.
    pub candidate: Option<CiStep>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CiPlan {
    pub steps: Vec<CiStep>,
    pub skips: Vec<CiSkip>,
    /// Setup actions (`actions/checkout`, `dtolnay/rust-toolchain`, ...) treated as no-ops.
    pub noops: usize,
    pub runner_temp: String,
}

/// Machine-specific inputs, injected so plans are testable.
#[derive(Debug, Clone, Default)]
pub struct PlanContext {
    pub runner_temp: PathBuf,
    pub workspace: PathBuf,
    /// Files changed against the base; `None` disables `paths:` filtering.
    pub changed_files: Option<Vec<String>>,
    /// The checked-out branch; `None` (detached or unknown) never matches a `branches:` filter.
    pub branch: Option<String>,
}

const NOOP_ACTIONS: &[&str] = &[
    "actions/checkout",
    "actions/setup-node",
    "actions/setup-python",
    "actions/setup-java",
    "actions/setup-go",
    "actions/setup-dotnet",
    "actions/cache",
    "dtolnay/rust-toolchain",
    "actions-rust-lang/setup-rust-toolchain",
    "swatinem/rust-cache",
    "pnpm/action-setup",
    "android-actions/setup-android",
    "gradle/actions/setup-gradle",
    "expo/expo-github-action",
];

const DEPLOY_MARKERS: &[&str] = &[
    "npm publish",
    "pnpm publish",
    "yarn publish",
    "cargo publish",
    "cargo login",
    "docker push",
    "docker login",
    "eas update",
    "eas build",
    "eas submit",
    "git push",
    "twine upload",
    "gh release",
    "gh pr ",
    "gh run ",
    "gh api ",
    "gh workflow",
    "gh issue",
    "gh auth",
    "fastlane",
    "altool",
    "wrangler",
    "aws ",
    "terraform ",
    "kubectl ",
    "helm ",
    "gcloud ",
    "az ",
    "flyctl",
    "fly deploy",
    "vercel",
    "netlify deploy",
    "firebase deploy",
    "heroku",
    "pulumi ",
    "deploy",
];

/// `git fetch`/`git pull` options that change how much history the clone holds. A step runs in
/// the owner's own clone, whose object store every worktree shares, so `--depth=1` there would
/// make the whole repository shallow.
const SHALLOW_FLAGS: &[&str] = &[
    "--depth",
    "--deepen",
    "--shallow-since",
    "--shallow-exclude",
    "--unshallow",
    "--update-shallow",
];

/// The first `git fetch`/`git pull` in `run` that changes the clone's history depth, as
/// `git fetch --depth`.
fn shallow_git(run: &str) -> Option<String> {
    run.split(['\n', ';', '&', '|']).find_map(|cmd| {
        let words: Vec<&str> = cmd.split_whitespace().collect();
        let git = words.iter().position(|w| *w == "git")?;
        let sub = words[git + 1..]
            .iter()
            .find(|w| matches!(**w, "fetch" | "pull"))?;
        let flag = words[git + 1..].iter().find_map(|w| {
            SHALLOW_FLAGS
                .iter()
                .find(|f| w.split('=').next() == Some(**f))
        })?;
        Some(format!("git {sub} {flag}"))
    })
}

const RUNNER_STATE: &[&str] = &[
    "GITHUB_ENV",
    "GITHUB_PATH",
    "GITHUB_OUTPUT",
    "GITHUB_STATE",
    "GITHUB_STEP_SUMMARY",
];

/// Read `.github/workflows/*.y{,a}ml` under `root` and build the plan.
pub fn build_plan(root: &Path, ctx: &PlanContext) -> CiPlan {
    let dir = root.join(".github").join("workflows");
    let mut sources = Vec::new();
    if let Ok(read) = std::fs::read_dir(&dir) {
        for entry in read.flatten() {
            let path = entry.path();
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if !matches!(ext, "yml" | "yaml") {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            sources.push((
                name,
                std::fs::read_to_string(&path).map_err(|e| e.to_string()),
            ));
        }
    }
    sources.sort_by(|a, b| a.0.cmp(&b.0));
    plan_from_sources(&sources, ctx, Some(root))
}

pub fn plan_from_sources(
    sources: &[(String, Result<String, String>)],
    ctx: &PlanContext,
    root: Option<&Path>,
) -> CiPlan {
    let mut plan = CiPlan {
        runner_temp: ctx.runner_temp.display().to_string(),
        ..CiPlan::default()
    };
    for (name, text) in sources {
        let text = match text {
            Ok(t) => t,
            Err(e) => {
                plan.workflow_skip(name, format!("unreadable: {e}"));
                continue;
            }
        };
        match yaml::parse(text) {
            Ok(doc) => plan_workflow(&mut plan, name, &doc, ctx, root),
            Err(e) => plan.workflow_skip(name, format!("unparsed: {e}")),
        }
    }
    plan
}

impl CiPlan {
    fn workflow_skip(&mut self, workflow: &str, reason: String) {
        self.skips.push(CiSkip {
            workflow: workflow.to_string(),
            job: None,
            step: None,
            reason,
            candidate: None,
        });
    }

    fn job_skip(&mut self, workflow: &str, job: &str, reason: String) {
        self.skips.push(CiSkip {
            workflow: workflow.to_string(),
            job: Some(job.to_string()),
            step: None,
            reason,
            candidate: None,
        });
    }
}

fn strings(y: Option<&Yaml>) -> Vec<String> {
    match y {
        Some(Yaml::Str(s)) => vec![s.clone()],
        Some(Yaml::Seq(items)) => items
            .iter()
            .filter_map(|i| i.as_str().map(str::to_string))
            .collect(),
        _ => Vec::new(),
    }
}

/// The events a workflow fires on, and the `paths` filters of push / pull_request.
/// `(paths, paths-ignore)` of one event.
type PathFilter = (Vec<String>, Vec<String>);

fn triggers(on: Option<&Yaml>, branch: Option<&str>) -> (Vec<String>, Vec<PathFilter>) {
    match on {
        Some(Yaml::Map(entries)) => {
            let events = entries.iter().map(|(k, _)| k.clone()).collect();
            let filters = entries
                .iter()
                .filter(|(k, v)| match k.as_str() {
                    "push" => push_fires(v, branch),
                    "pull_request" | "pull_request_target" => true,
                    _ => false,
                })
                .map(|(_, v)| (strings(v.get("paths")), strings(v.get("paths-ignore"))))
                .collect();
            (events, filters)
        }
        other => (strings(other), Vec::new()),
    }
}

/// Whether a `push:` trigger fires for a push of `branch`: a tags-only trigger never does, and a
/// `branches` filter needs the branch to match, as the runner would decide.
fn push_fires(push: &Yaml, branch: Option<&str>) -> bool {
    let branches = strings(push.get("branches"));
    let ignore = strings(push.get("branches-ignore"));
    let has_tags = push.get("tags").is_some() || push.get("tags-ignore").is_some();
    if branches.is_empty() && ignore.is_empty() {
        return !has_tags;
    }
    let Some(b) = branch else {
        return false;
    };
    if !ignore.is_empty() {
        return !ignore.iter().any(|p| glob_match(p, b));
    }
    let mut hit = false;
    for p in &branches {
        match p.strip_prefix('!') {
            Some(neg) if glob_match(neg, b) => hit = false,
            None if glob_match(p, b) => hit = true,
            _ => {}
        }
    }
    hit
}

/// A job-level `if:` that depends on the ref or event, which local runs cannot reproduce.
fn ref_or_event_bound(cond: &str) -> bool {
    cond.contains("github.ref") || cond.contains("github.event")
}

fn plan_workflow(
    plan: &mut CiPlan,
    name: &str,
    doc: &Yaml,
    ctx: &PlanContext,
    root: Option<&Path>,
) {
    let (events, filters) = triggers(doc.get("on"), ctx.branch.as_deref());
    let push_ok = doc
        .get("on")
        .and_then(|on| on.get("push"))
        .is_none_or(|v| push_fires(v, ctx.branch.as_deref()));
    let fires = |e: &str| events.iter().any(|x| x == e) && (e != "push" || push_ok);
    if !(fires("push") || fires("pull_request") || fires("pull_request_target")) {
        plan.workflow_skip(
            name,
            format!(
                "does not run on push or pull_request (on: {})",
                events.join(", ")
            ),
        );
        return;
    }
    if let Some(changed) = &ctx.changed_files {
        let matches_any = filters.is_empty()
            || filters.iter().any(|(paths, ignore)| {
                changed.iter().any(|f| {
                    (paths.is_empty() || paths.iter().any(|p| glob_match(p, f)))
                        && !ignore.iter().any(|p| glob_match(p, f))
                })
            });
        if !matches_any {
            plan.workflow_skip(name, "path filter: no changed file matches".to_string());
            return;
        }
    }
    let wf_env = env_map(doc.get("env"));
    let defaults = doc.get("defaults").and_then(|d| d.get("run"));
    let Some(jobs) = doc.get("jobs").and_then(Yaml::as_map) else {
        plan.workflow_skip(name, "unparsed: no jobs".to_string());
        return;
    };
    for (job_id, job) in jobs {
        plan_job(plan, name, job_id, job, &wf_env, defaults, ctx, root);
    }
}

fn env_map(y: Option<&Yaml>) -> Vec<(String, String)> {
    y.and_then(Yaml::as_map)
        .map(|m| {
            m.iter()
                .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
                .collect()
        })
        .unwrap_or_default()
}

type Matrix = Vec<(String, String)>;

/// Expand `strategy.matrix` (plain axes of scalar lists only) into combinations.
fn expand_matrix(job: &Yaml) -> Result<Vec<Matrix>, String> {
    let Some(matrix) = job.get("strategy").and_then(|s| s.get("matrix")) else {
        return Ok(vec![Vec::new()]);
    };
    let Some(axes) = matrix.as_map() else {
        return Err("matrix is computed (not a plain list)".to_string());
    };
    let mut combos: Vec<Matrix> = vec![Vec::new()];
    for (axis, values) in axes {
        if axis == "include" || axis == "exclude" {
            return Err(format!("matrix `{axis}` is not expanded"));
        }
        let Some(values) = values.as_seq() else {
            return Err(format!("matrix axis `{axis}` is not a plain list"));
        };
        let values: Vec<&str> = values.iter().filter_map(Yaml::as_str).collect();
        combos = combos
            .into_iter()
            .flat_map(|c| {
                values.iter().map(move |v| {
                    let mut next = c.clone();
                    next.push((axis.clone(), (*v).to_string()));
                    next
                })
            })
            .collect();
        if combos.len() > 16 {
            return Err("matrix has more than 16 combinations".to_string());
        }
    }
    Ok(combos)
}

struct Scope<'a> {
    matrix: &'a Matrix,
    env: &'a [(String, String)],
    ctx: &'a PlanContext,
}

/// Substitute `${{ ... }}` expressions we can know; otherwise say why the text is unusable.
fn resolve(text: &str, scope: &Scope) -> Result<String, String> {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find("${{") {
        out.push_str(&rest[..at]);
        let Some(end) = rest[at..].find("}}") else {
            return Err("unterminated `${{` expression".to_string());
        };
        let expr = rest[at + 3..at + end].trim();
        rest = &rest[at + end + 2..];
        let known = if let Some(axis) = expr.strip_prefix("matrix.") {
            scope
                .matrix
                .iter()
                .find(|(k, _)| k == axis)
                .map(|(_, v)| v.clone())
        } else if let Some(var) = expr.strip_prefix("env.") {
            scope
                .env
                .iter()
                .find(|(k, _)| k == var)
                .map(|(_, v)| v.clone())
        } else {
            match expr {
                "runner.temp" => Some(scope.ctx.runner_temp.display().to_string()),
                "runner.os" => Some("Linux".to_string()),
                "runner.arch" => Some("X64".to_string()),
                "github.workspace" => Some(scope.ctx.workspace.display().to_string()),
                _ => None,
            }
        };
        match known {
            Some(v) if !v.contains("${{") => out.push_str(&v),
            _ => return Err(unresolved_reason(expr)),
        }
    }
    out.push_str(rest);
    Ok(out)
}

fn unresolved_reason(expr: &str) -> String {
    if let Some(name) = expr.strip_prefix("secrets.") {
        format!("needs secret {name}")
    } else if expr == "github.token" {
        "needs GITHUB_TOKEN".to_string()
    } else if let Some(name) = expr.strip_prefix("vars.") {
        format!("needs repository variable {name}")
    } else {
        format!("needs GitHub context `${{{{ {expr} }}}}`")
    }
}

fn is_linux(labels: &[String]) -> Result<(), String> {
    for l in labels {
        let l = l.to_ascii_lowercase();
        if l.contains("windows") {
            return Err(format!("runs on {l} (not Linux)"));
        }
        if l.contains("macos") {
            return Err(format!("runs on {l} (not Linux)"));
        }
    }
    Ok(())
}

/// Evaluate the few `if:` forms we know; `None` when the condition depends on CI-only state.
fn eval_if(cond: &str) -> Option<bool> {
    let c = cond.trim();
    let c = c
        .strip_prefix("${{")
        .and_then(|x| x.strip_suffix("}}"))
        .map_or(c, str::trim);
    match c {
        "success()" | "true" => Some(true),
        "false" => Some(false),
        "runner.os == 'Linux'" | "runner.os != 'Windows'" | "runner.os != 'macOS'" => Some(true),
        "runner.os == 'Windows'" | "runner.os == 'macOS'" => Some(false),
        _ => None,
    }
}

fn only_scheduled(cond: &str) -> bool {
    cond.contains("github.event_name")
        && !cond.contains("'push'")
        && !cond.contains("'pull_request'")
        && !cond.contains("!=")
}

#[allow(clippy::too_many_arguments)]
fn plan_job(
    plan: &mut CiPlan,
    wf: &str,
    job_id: &str,
    job: &Yaml,
    wf_env: &[(String, String)],
    defaults: Option<&Yaml>,
    ctx: &PlanContext,
    root: Option<&Path>,
) {
    if let Some(cond) = job.get("if").and_then(Yaml::as_str) {
        if only_scheduled(cond) {
            plan.job_skip(
                wf,
                job_id,
                format!("does not run on push or pull_request (if: {cond})"),
            );
            return;
        }
        if eval_if(cond) == Some(false) {
            plan.job_skip(
                wf,
                job_id,
                format!("condition is false locally (if: {cond})"),
            );
            return;
        }
        if eval_if(cond).is_none() && ref_or_event_bound(cond) {
            plan.job_skip(wf, job_id, format!("conditional job (if: {cond})"));
            return;
        }
    }
    if job.get("uses").is_some() {
        plan.job_skip(wf, job_id, "calls a reusable workflow".to_string());
        return;
    }
    if job.get("container").is_some() {
        plan.job_skip(wf, job_id, "runs in a container".to_string());
        return;
    }
    if job.get("services").is_some() {
        plan.job_skip(wf, job_id, "needs service containers".to_string());
        return;
    }
    if job.get("environment").is_some() {
        plan.job_skip(wf, job_id, "deployment environment".to_string());
        return;
    }
    let combos = match expand_matrix(job) {
        Ok(c) => c,
        Err(why) => {
            plan.job_skip(wf, job_id, why);
            return;
        }
    };
    let runs_on_raw = match job.get("runs-on") {
        Some(Yaml::Map(_)) => strings(job.get("runs-on").and_then(|r| r.get("labels"))),
        other => strings(other),
    };
    for matrix in &combos {
        let label = if matrix.is_empty() {
            job_id.to_string()
        } else {
            let axes: Vec<String> = matrix.iter().map(|(k, v)| format!("{k}={v}")).collect();
            format!("{job_id} [{}]", axes.join(", "))
        };
        let scope = Scope {
            matrix,
            env: &[],
            ctx,
        };
        let labels: Result<Vec<String>, String> =
            runs_on_raw.iter().map(|l| resolve(l, &scope)).collect();
        match labels.and_then(|l| is_linux(&l).map(|()| l)) {
            Ok(_) => {}
            Err(why) => {
                plan.job_skip(wf, &label, why);
                continue;
            }
        }
        plan_steps(plan, wf, &label, job, matrix, wf_env, defaults, ctx, root);
    }
}

#[allow(clippy::too_many_arguments)]
fn plan_steps(
    plan: &mut CiPlan,
    wf: &str,
    label: &str,
    job: &Yaml,
    matrix: &Matrix,
    wf_env: &[(String, String)],
    defaults: Option<&Yaml>,
    ctx: &PlanContext,
    root: Option<&Path>,
) {
    let job_defaults = job.get("defaults").and_then(|d| d.get("run")).or(defaults);
    let def_shell = job_defaults
        .and_then(|d| d.get("shell"))
        .and_then(Yaml::as_str);
    let def_dir = job_defaults
        .and_then(|d| d.get("working-directory"))
        .and_then(Yaml::as_str);
    let timeout = |y: &Yaml| {
        y.get("timeout-minutes")
            .and_then(Yaml::as_str)
            .and_then(|t| t.parse::<u64>().ok())
            .map(|m| m * 60)
    };
    // Workflow + job env, resolved in order; what cannot be known is remembered by name.
    let mut base: Vec<(String, String)> = Vec::new();
    let mut unknown: Vec<(String, String)> = Vec::new();
    for (k, v) in wf_env.iter().cloned().chain(env_map(job.get("env"))) {
        match resolve(
            &v,
            &Scope {
                matrix,
                env: &base,
                ctx,
            },
        ) {
            Ok(val) => {
                base.retain(|(n, _)| *n != k);
                base.push((k, val));
            }
            Err(why) => unknown.push((k, why)),
        }
    }
    let Some(steps) = job.get("steps").and_then(Yaml::as_seq) else {
        return;
    };
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for step in steps {
        if let Some(uses) = step.get("uses").and_then(Yaml::as_str) {
            let lower = uses.to_ascii_lowercase();
            let action = lower.split('@').next().unwrap_or("");
            if NOOP_ACTIONS.contains(&action) {
                plan.noops += 1;
            } else {
                plan.skips.push(CiSkip {
                    workflow: wf.to_string(),
                    job: Some(label.to_string()),
                    step: Some(
                        step.get("name")
                            .and_then(Yaml::as_str)
                            .unwrap_or(uses)
                            .to_string(),
                    ),
                    reason: format!("uses: {action} (not runnable locally)"),
                    candidate: None,
                });
            }
            continue;
        }
        let Some(run) = step.get("run").and_then(Yaml::as_str) else {
            continue;
        };
        let first_line = run.lines().find(|l| !l.trim().is_empty()).unwrap_or("run");
        let mut name = step.get("name").and_then(Yaml::as_str).map_or_else(
            || first_line.chars().take(60).collect::<String>(),
            str::to_string,
        );
        let n = seen.entry(name.clone()).or_insert(0);
        *n += 1;
        if *n > 1 {
            name = format!("{name} #{n}");
        }
        let mut cand = CiStep {
            key: format!("{wf}::{label}::{name}"),
            workflow: wf.to_string(),
            job: label.to_string(),
            name: name.clone(),
            run: run.to_string(),
            env: BTreeMap::new(),
            cwd: step
                .get("working-directory")
                .and_then(Yaml::as_str)
                .or(def_dir)
                .map(str::to_string),
            shell: step
                .get("shell")
                .and_then(Yaml::as_str)
                .or(def_shell)
                .map(str::to_string),
            timeout_secs: timeout(step),
            allow_failure: step.get("continue-on-error").and_then(Yaml::as_str) == Some("true"),
        };
        let reason = skip_reason(&mut cand, step, matrix, &base, &unknown, ctx, root);
        match reason {
            None => plan.steps.push(cand),
            Some(reason) => plan.skips.push(CiSkip {
                workflow: wf.to_string(),
                job: Some(label.to_string()),
                step: Some(name),
                reason,
                candidate: Some(cand),
            }),
        }
    }
}

/// Fill in `cand`'s resolved script and env; the reason it cannot run here, if any.
fn skip_reason(
    cand: &mut CiStep,
    step: &Yaml,
    matrix: &Matrix,
    base: &[(String, String)],
    unknown: &[(String, String)],
    ctx: &PlanContext,
    root: Option<&Path>,
) -> Option<String> {
    let mut env: Vec<(String, String)> = base.to_vec();
    let mut reason = None;
    for (k, v) in env_map(step.get("env")) {
        match resolve(
            &v,
            &Scope {
                matrix,
                env: &env,
                ctx,
            },
        ) {
            Ok(val) => {
                env.retain(|(n, _)| *n != k);
                env.push((k, val));
            }
            Err(why) => reason = reason.or(Some(format!("env {k}: {why}"))),
        }
    }
    cand.env = env.iter().cloned().collect();
    match resolve(
        &cand.run,
        &Scope {
            matrix,
            env: &env,
            ctx,
        },
    ) {
        Ok(run) => cand.run = run,
        Err(why) => reason = reason.or(Some(why)),
    }
    if let Some(cond) = step.get("if").and_then(Yaml::as_str) {
        let c = cond.trim();
        if c.contains("always()") || c.contains("cancelled()") || c.contains("failure()") {
            reason = reason.or(Some(format!("cleanup step (if: {c})")));
        } else if eval_if(c) == Some(false) {
            reason = reason.or(Some(format!("condition is false locally (if: {c})")));
        } else if eval_if(c).is_none() {
            reason = reason.or(Some(format!("conditional step (if: {c})")));
        }
    }
    if let Some(shell) = cand.shell.as_deref() {
        if !matches!(shell, "bash" | "sh") {
            reason = reason.or(Some(format!("shell: {shell}")));
        }
    }
    if let Some((var, why)) = unknown.iter().find(|(k, _)| mentions_var(&cand.run, k)) {
        reason = reason.or(Some(format!("env {var}: {why}")));
    }
    if let Some(m) = DEPLOY_MARKERS.iter().find(|m| cand.run.contains(**m)) {
        reason = reason.or(Some(format!("network/deploy command (`{}`)", m.trim())));
    }
    if let Some(cmd) = shallow_git(&cand.run) {
        reason = reason.or(Some(format!(
            "would make the shared clone shallow (`{cmd}`)"
        )));
    }
    if names_runner_state(&cand.run) || root.is_some_and(|r| script_touches_runner(r, cand)) {
        reason = reason.or(Some(
            "writes GitHub runner state ($GITHUB_ENV/$GITHUB_OUTPUT)".to_string(),
        ));
    }
    reason
}

/// Text that touches one of the runner's state files (`$GITHUB_ENV`, `$GITHUB_OUTPUT`, ...).
fn names_runner_state(text: &str) -> bool {
    RUNNER_STATE.iter().any(|name| text.contains(name))
}

fn mentions_var(run: &str, name: &str) -> bool {
    run.contains(&format!("${name}")) || run.contains(&format!("${{{name}}}"))
}

/// A repo script named in the command that itself writes runner state.
fn script_touches_runner(root: &Path, cand: &CiStep) -> bool {
    let base = cand
        .cwd
        .as_deref()
        .map_or_else(|| root.to_path_buf(), |c| root.join(c));
    cand.run
        .split_whitespace()
        .filter(|w| w.ends_with(".sh") || w.ends_with(".py"))
        .any(|w| {
            let path = base.join(w.trim_matches(|c| c == '"' || c == '\''));
            // A test of a runner-state script only names the variable; it does not write it.
            let is_test = path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("test"));
            !is_test
                && std::fs::metadata(&path).is_ok_and(|m| m.len() < 1_000_000)
                && std::fs::read_to_string(&path).is_ok_and(|t| names_runner_state(&t))
        })
}

/// GitHub-style path glob: `*` stays inside a path segment, `**` crosses them.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    fn go(p: &[u8], t: &[u8]) -> bool {
        match p.first() {
            None => t.is_empty(),
            Some(b'*') if p.get(1) == Some(&b'*') => {
                let rest = p[2..].strip_prefix(b"/").unwrap_or(&p[2..]);
                (0..=t.len()).any(|i| go(rest, &t[i..]))
            }
            Some(b'*') => (0..=t.len())
                .take_while(|&i| i == 0 || t[i - 1] != b'/')
                .any(|i| go(&p[1..], &t[i..])),
            Some(b'?') => !t.is_empty() && t[0] != b'/' && go(&p[1..], &t[1..]),
            Some(c) => t.first() == Some(c) && go(&p[1..], &t[1..]),
        }
    }
    go(pattern.as_bytes(), text.as_bytes())
}

#[cfg(test)]
mod tests;
