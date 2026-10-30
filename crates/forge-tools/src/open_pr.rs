//! `open_pr`: turn the committed work on the current branch into a GitHub pull request without the
//! model composing any of the mechanics.
//!
//! The tool refuses a dirty tree, a detached HEAD, the default branch, and a remote the signed-in
//! `gh` user cannot push to (`gh repo view --json viewerPermission`). It then pushes the branch
//! (never with force), publishes the session's screenshots to the orphan `forge-evidence` branch,
//! and runs `gh pr create` with the model's title and summary followed by the generated Evidence
//! (passing checks, screenshots, live preview URLs) and Provenance sections. The model fills in
//! neither section, so it cannot invent proof.
//!
//! The whole call is [`SideEffect::External`]: the permission broker asks before it runs in the
//! guarded modes and denies it in plan mode, on the direct path and the CLI bridge alike.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use forge_types::SideEffect;
use serde_json::{json, Value};

use crate::evidence::{self, EvidenceKind, Ledger, PrBlockOptions};
use crate::{Tool, ToolError};

mod command;
mod evidence_branch;
mod github;
mod repo_state;
mod screenshots;

use command::{run, run_raw, run_with_stdin};
use github::Repo;
use repo_state::{dirty_paths, Dirty};

pub const OPEN_PR_TOOL: &str = "open_pr";

/// Builds the provenance section for a session: `(session id, diff stat)`.
pub type ProvenanceFn = Arc<dyn Fn(&str, Option<&str>) -> String + Send + Sync>;
/// Masks secrets in text bound for a public PR.
pub type RedactFn = Arc<dyn Fn(&str) -> String + Send + Sync>;
/// A directory's own `[pr] publish_evidence_images` (its `.forge/config.toml`), if it sets one.
pub type RepoFlagFn = Arc<dyn Fn(&Path) -> Option<bool> + Send + Sync>;

/// What the binary installs at startup: `[pr]` config plus the two pieces that need the store and
/// the secret list, which this crate cannot reach.
#[derive(Clone)]
pub struct OpenPrSettings {
    /// `[pr] publish_evidence_images`: push screenshots to the `forge-evidence` branch.
    pub publish_images: bool,
    /// Read at call time, because a daemon serves many repositories and loaded its config from
    /// none of them: `false` in the workspace's or repository's `.forge/config.toml` wins.
    pub repo_publish_images: Option<RepoFlagFn>,
    pub provenance: Option<ProvenanceFn>,
    pub redact: Option<RedactFn>,
}

impl Default for OpenPrSettings {
    fn default() -> Self {
        Self {
            publish_images: true,
            repo_publish_images: None,
            provenance: None,
            redact: None,
        }
    }
}

static SETTINGS: Mutex<Option<OpenPrSettings>> = Mutex::new(None);

pub fn set_open_pr_settings(settings: OpenPrSettings) {
    *SETTINGS.lock().unwrap_or_else(PoisonError::into_inner) = Some(settings);
}

fn settings() -> OpenPrSettings {
    SETTINGS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .unwrap_or_default()
}

#[derive(Default)]
pub struct OpenPrTool {
    git: Option<PathBuf>,
    gh: Option<PathBuf>,
}

impl OpenPrTool {
    /// A tool that runs these binaries instead of `git`/`gh` from PATH (tests).
    pub fn with_binaries(git: PathBuf, gh: PathBuf) -> Self {
        Self {
            git: Some(git),
            gh: Some(gh),
        }
    }
}

struct Args {
    title: String,
    body: String,
    base: Option<String>,
    draft: bool,
    allow_untracked: bool,
    publish_images: bool,
}

impl Args {
    fn parse(args: &Value) -> Result<Self, ToolError> {
        let text = |key: &str| {
            args.get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        let flag =
            |key: &str, default: bool| args.get(key).and_then(Value::as_bool).unwrap_or(default);
        let title = text("title").ok_or_else(|| ToolError::BadArgs("title is required".into()))?;
        if title.contains('\n') {
            return Err(ToolError::BadArgs("title must be one line".into()));
        }
        Ok(Self {
            title,
            body: text("body").unwrap_or_default(),
            base: text("base"),
            draft: flag("draft", false),
            allow_untracked: flag("allow_untracked", false),
            publish_images: flag("publish_images", true),
        })
    }
}

#[async_trait]
impl Tool for OpenPrTool {
    fn name(&self) -> &str {
        OPEN_PR_TOOL
    }
    fn description(&self) -> &str {
        "Open a GitHub pull request for the current branch. Commit your work on a feature branch \
         first: the tool refuses uncommitted changes, the default branch, and remotes you cannot \
         push to. It pushes the branch (never force), publishes this session's screenshots to the \
         repo's `forge-evidence` branch (publish_images:false skips that), and runs `gh pr create` \
         with your title and body followed by generated Evidence (passing checks, screenshots, \
         live preview URLs) and Provenance sections — do not write those yourself. If the branch \
         already has an open PR it pushes and returns that PR. Returns the PR URL. Asks the user \
         first."
    }
    fn side_effect(&self) -> SideEffect {
        SideEffect::External
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "PR title, one line (conventional-commit style)." },
                "body": { "type": "string", "description": "What changed and why, in Markdown. Evidence and provenance are appended for you." },
                "base": { "type": "string", "description": "Base branch. Default: the repository's default branch." },
                "draft": { "type": "boolean", "description": "Open as a draft PR." },
                "allow_untracked": { "type": "boolean", "description": "Proceed even though untracked files exist (they are not part of the PR)." },
                "publish_images": { "type": "boolean", "description": "Push captured screenshots to the forge-evidence branch so they render in the PR (default true)." }
            },
            "required": ["title", "body"]
        })
    }
    async fn run(&self, args: &Value) -> Result<String, ToolError> {
        let args = Args::parse(args)?;
        let workspace = crate::SESSION_WORKSPACE
            .try_with(Clone::clone)
            .ok()
            .or_else(|| std::env::current_dir().ok())
            .ok_or_else(|| ToolError::Failed("no workspace to open a PR from".into()))?;
        let ctx = Ctx {
            git: self.git.clone().unwrap_or_else(|| PathBuf::from("git")),
            gh: self.gh.clone().unwrap_or_else(|| PathBuf::from("gh")),
            settings: settings(),
            session: evidence::current_session(),
            ledger: Ledger::current(),
        };
        open_pr(&ctx, &workspace, &args)
            .await
            .map_err(ToolError::Failed)
    }
}

/// Create the PR for `workspace`'s checked-out branch with the same refusals, push, evidence and
/// provenance the model-facing tool applies. For daemon routes where a person, not the model,
/// composed the title and body.
pub async fn create_pr_in(workspace: &Path, args: &Value) -> Result<String, String> {
    let args = Args::parse(args).map_err(|e| e.to_string())?;
    let ctx = Ctx {
        git: PathBuf::from("git"),
        gh: PathBuf::from("gh"),
        settings: settings(),
        session: None,
        ledger: None,
    };
    open_pr(&ctx, workspace, &args).await
}

struct Ctx {
    git: PathBuf,
    gh: PathBuf,
    settings: OpenPrSettings,
    session: Option<String>,
    ledger: Option<Ledger>,
}

async fn open_pr(ctx: &Ctx, workspace: &Path, args: &Args) -> Result<String, String> {
    let git = ctx.git.as_path();
    let top = PathBuf::from(
        run(git, &["rev-parse", "--show-toplevel"], workspace, &[])
            .await
            .map_err(|e| format!("not inside a git repository: {e}"))?,
    );
    check_clean(git, &top, args.allow_untracked).await?;
    let branch = run(git, &["symbolic-ref", "--short", "-q", "HEAD"], &top, &[])
        .await
        .map_err(|_| "HEAD is detached; check out a feature branch first".to_string())?;
    let remote = push_remote(git, &top, &branch).await?;
    let repo = github_repo(git, &top, &remote).await?;
    let slug = repo.slug();
    let info = github::parse_repo_view(
        &run(
            &ctx.gh,
            &[
                "repo",
                "view",
                &slug,
                "--json",
                "viewerPermission,defaultBranchRef,isArchived",
            ],
            &top,
            &[],
        )
        .await?,
    )?;
    if info.archived {
        return Err(format!(
            "{slug} is archived; it cannot take new pull requests"
        ));
    }
    if !github::can_push(&info.permission) {
        return Err(format!(
            "the signed-in gh user has {} access to {slug}, not write access; refusing to push. \
             Fork the repository and point the remote at the fork, or ask the user to open the PR.",
            if info.permission.is_empty() {
                "no"
            } else {
                &info.permission
            }
        ));
    }
    let base = args.base.clone().unwrap_or(info.default_branch);
    if base.is_empty() {
        return Err(format!("{slug} reports no default branch; pass `base`"));
    }
    if branch == base {
        return Err(format!(
            "on `{base}`, the base branch itself; create a feature branch (git switch -c …) and \
             commit there first"
        ));
    }
    let base_ref = format!("refs/remotes/{remote}/{base}");
    let has_base_ref = run_raw(
        git,
        &["rev-parse", "--verify", "-q", &base_ref],
        &top,
        &[],
        None,
    )
    .await?
    .ok;
    let ahead = if has_base_ref {
        let count = run(
            git,
            &["rev-list", "--count", &format!("{base_ref}..HEAD")],
            &top,
            &[],
        )
        .await?;
        let count = count.trim().parse::<u64>().unwrap_or(0);
        if count == 0 {
            return Err(format!(
                "`{branch}` has no commits ahead of {remote}/{base}; commit the work first"
            ));
        }
        Some(count)
    } else {
        None
    };
    let existing = github::first_pr_url(
        &run(
            &ctx.gh,
            &[
                "pr",
                "list",
                "--repo",
                &slug,
                "--head",
                &branch,
                "--state",
                "open",
                "--json",
                "url,isCrossRepository",
                "--limit",
                "20",
            ],
            &top,
            &[],
        )
        .await?,
    );

    run(
        git,
        &[
            "push",
            "--quiet",
            "-u",
            &remote,
            &format!("HEAD:refs/heads/{branch}"),
        ],
        &top,
        &[],
    )
    .await
    .map_err(|e| format!("push refused (no force push is attempted): {e}"))?;
    if let Some(url) = existing {
        return Ok(format!(
            "pushed `{branch}` to the open PR {url} (title and body left as they are)"
        ));
    }

    let mut notes: Vec<String> = Vec::new();
    let images = publish_images(ctx, args, workspace, &top, &remote, &repo, &mut notes).await;
    let live = crate::shell::live_preview_urls(workspace);
    let diff_stat = if has_base_ref {
        run(
            git,
            &["diff", "--shortstat", &format!("{base_ref}...HEAD")],
            &top,
            &[],
        )
        .await
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    } else {
        None
    };
    let body = compose_body(
        ctx,
        &args.body,
        images.as_ref(),
        &live,
        diff_stat.as_deref(),
    );
    let redact = |text: &str| match &ctx.settings.redact {
        Some(redact) => redact(text),
        None => text.to_string(),
    };
    let title = redact(&args.title);
    let body = redact(&body);
    let mut create = vec![
        "pr",
        "create",
        "--repo",
        &slug,
        "--base",
        &base,
        "--head",
        &branch,
        "--title",
        &title,
        "--body-file",
        "-",
    ];
    if args.draft {
        create.push("--draft");
    }
    let out = run_with_stdin(&ctx.gh, &create, &top, &[], Some(&body)).await?;
    let url = github::created_pr_url(&out).unwrap_or(out);
    let mut text = format!(
        "opened PR {url}\n  `{branch}` → `{base}` on {slug}{}",
        ahead.map_or(String::new(), |n| format!(", {n} commit(s)"))
    );
    text.push_str(&format!(
        "\n  {}",
        evidence_summary(ctx, images.as_ref(), &live)
    ));
    for note in notes {
        text.push_str(&format!("\n  note: {note}"));
    }
    Ok(text)
}

async fn check_clean(git: &Path, top: &Path, allow_untracked: bool) -> Result<(), String> {
    let status = run(git, &["status", "--porcelain=v1", "-z"], top, &[]).await?;
    let Dirty { tracked, untracked } = dirty_paths(&status);
    if !tracked.is_empty() {
        return Err(format!(
            "uncommitted changes in {}; commit them (stage specific paths) or stash them first",
            list(&tracked)
        ));
    }
    if !untracked.is_empty() && !allow_untracked {
        return Err(format!(
            "untracked files: {}. Commit the ones that belong in the PR, or pass \
             allow_untracked:true if none do",
            list(&untracked)
        ));
    }
    Ok(())
}

fn list(paths: &[String]) -> String {
    const SHOWN: usize = 8;
    let mut text = paths
        .iter()
        .take(SHOWN)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    if paths.len() > SHOWN {
        text.push_str(&format!(" and {} more", paths.len() - SHOWN));
    }
    text
}

/// The GitHub repository behind `remote`: its configured URL, else the URL git resolves it to
/// (`insteadOf` applied) when the configured one is a shorthand such as `gh:owner/repo`, whose
/// dotless "host" is an alias rather than a GitHub host.
async fn github_repo(git: &Path, top: &Path, remote: &str) -> Result<Repo, String> {
    let configured = remote_url(git, top, remote).await?;
    if let Some(repo) = github::parse_remote_url(&configured).filter(|r| r.host.contains('.')) {
        return Ok(repo);
    }
    run(git, &["remote", "get-url", "--push", remote], top, &[])
        .await
        .ok()
        .and_then(|resolved| github::parse_remote_url(&resolved))
        .ok_or_else(|| format!("remote `{remote}` ({configured}) is not a GitHub repository URL"))
}

/// The remote's URL as configured (`pushurl`, else `url`), before `insteadOf` rewriting: that is
/// the name the user gave the repository, which is what `gh` needs.
async fn remote_url(git: &Path, top: &Path, remote: &str) -> Result<String, String> {
    for key in [
        format!("remote.{remote}.pushurl"),
        format!("remote.{remote}.url"),
    ] {
        if let Ok(url) = run(git, &["config", "--get", &key], top, &[]).await {
            if !url.trim().is_empty() {
                return Ok(url.trim().to_string());
            }
        }
    }
    Err(format!("remote `{remote}` has no URL"))
}

/// The branch's configured push remote, else `origin`, else the only remote.
async fn push_remote(git: &Path, top: &Path, branch: &str) -> Result<String, String> {
    for key in [
        format!("branch.{branch}.pushRemote"),
        "remote.pushDefault".to_string(),
        format!("branch.{branch}.remote"),
    ] {
        if let Ok(remote) = run(git, &["config", "--get", &key], top, &[]).await {
            let remote = remote.trim();
            if !remote.is_empty() && remote != "." {
                return Ok(remote.to_string());
            }
        }
    }
    let remotes = run(git, &["remote"], top, &[]).await?;
    let remotes: Vec<&str> = remotes
        .lines()
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .collect();
    match remotes.as_slice() {
        [] => Err("this repository has no git remote to push to".to_string()),
        [only] => Ok((*only).to_string()),
        many if many.contains(&"origin") => Ok("origin".to_string()),
        many => Err(format!(
            "several remotes ({}) and none is `origin` or configured for `{branch}`; set \
             `git branch --set-upstream-to` or `remote.pushDefault`",
            many.join(", ")
        )),
    }
}

async fn publish_images(
    ctx: &Ctx,
    args: &Args,
    workspace: &Path,
    top: &Path,
    remote: &str,
    repo: &Repo,
    notes: &mut Vec<String>,
) -> Option<evidence_branch::Published> {
    let (Some(session), Some(ledger)) = (&ctx.session, &ctx.ledger) else {
        return None;
    };
    let screenshots::Selection { files, refused } = screenshots::publishable(ledger);
    if !refused.is_empty() {
        notes.push(format!(
            "not published (not an intact capture inside the evidence directory): {}",
            list(&refused)
        ));
    }
    if files.is_empty() {
        return None;
    }
    let repo_allows = |dir: &Path| {
        ctx.settings
            .repo_publish_images
            .as_ref()
            .and_then(|flag| flag(dir))
            .unwrap_or(true)
    };
    let allowed = ctx.settings.publish_images
        && args.publish_images
        && repo_allows(workspace)
        && repo_allows(top);
    if !allowed {
        notes.push("screenshots were not published; the PR names them by local path".into());
        return None;
    }
    match evidence_branch::publish(&ctx.git, top, remote, repo, session, &files).await {
        Ok(published) => Some(published),
        Err(error) => {
            notes.push(format!(
                "screenshots could not be published to {} ({error}); the PR names them by local path",
                evidence_branch::BRANCH
            ));
            None
        }
    }
}

fn compose_body(
    ctx: &Ctx,
    summary: &str,
    images: Option<&evidence_branch::Published>,
    live: &[String],
    diff_stat: Option<&str>,
) -> String {
    let mut body = summary.trim().to_string();
    if !body.is_empty() {
        body.push_str("\n\n");
    }
    body.push_str("---\n");
    let Some(session) = &ctx.session else {
        body.push_str("🔨 Created with Forge\n");
        return body;
    };
    let options = PrBlockOptions {
        image_urls: images.map(|p| &p.urls),
        live_previews: Some(live),
    };
    if let Some(block) = ctx.ledger.as_ref().and_then(|l| l.pr_block(&options)) {
        body.push_str(&block);
        body.push('\n');
    }
    match &ctx.settings.provenance {
        Some(provenance) => body.push_str(&provenance(session, diff_stat)),
        None => {
            body.push_str(&format!("### Provenance\n- Session: `{session}`\n"));
            if let Some(diff) = diff_stat {
                body.push_str(&format!("- Diff: {diff}\n"));
            }
            body.push_str("\n🔨 Created with Forge\n");
        }
    }
    body
}

fn evidence_summary(
    ctx: &Ctx,
    images: Option<&evidence_branch::Published>,
    live: &[String],
) -> String {
    let items = ctx.ledger.as_ref().map(Ledger::items).unwrap_or_default();
    let count = |kind: EvidenceKind| items.iter().filter(|i| i.kind == kind).count();
    let previews = items
        .iter()
        .filter(|i| i.kind == EvidenceKind::Preview)
        .filter_map(|i| i.url.as_ref())
        .filter(|u| live.contains(u))
        .collect::<std::collections::HashSet<_>>()
        .len();
    let shots = match images {
        Some(published) => format!(
            "{} screenshot(s) published to {}@{}",
            published.urls.len(),
            evidence_branch::BRANCH,
            &published.commit[..published.commit.len().min(10)]
        ),
        None => format!("{} screenshot(s)", count(EvidenceKind::Screenshot)),
    };
    format!(
        "evidence: {} passing check(s), {shots}, {previews} live preview URL(s)",
        count(EvidenceKind::TestOutput)
    )
}

#[cfg(all(test, unix))]
mod tests;
