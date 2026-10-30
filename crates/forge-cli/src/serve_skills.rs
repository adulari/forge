//! `forge serve`'s skills surface: the catalog with usage counts, marketplace browsing and
//! installs, removal, the sync into `.agents/skills` / `.claude/skills`, and the invoke form.
//!
//! Routes (token-scoped under `{base}`):
//! - `GET  /api/skills`                       catalog rows with `uses` and the installing `pack`
//! - `GET  /api/skills/state`                 marketplaces, installed packs and the sync targets
//! - `GET  /api/skills/available?marketplace=` packages a marketplace offers (clones it)
//! - `POST /api/skills/install`   `{source, scope, marketplace?, session?}`
//! - `POST /api/skills/remove`    `{name, scope, session?}`
//! - `POST /api/skills/marketplaces` `{name, source, ref?}` / `POST /api/skills/marketplaces/remove`
//! - `POST /api/skills/sync`      `{scope, session?}` symlink skills where bridged agents look
//! - `POST /api/skills/invoke`    `{name, task?}` count the use and return the `/skill` line to send
//!
//! A client never names a directory: the project scope resolves through a session's repository.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::{Json, Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;
use serde::{Deserialize, Serialize};

use crate::cli::commands::marketplace::{
    add_marketplace_at, install_pack_to, installed_path, list_available, load_installed_at,
    load_marketplaces_at, marketplaces_path, remove_marketplace_at, remove_plugin_at, skills_dir,
    MarketplaceEntry, Marketplaces,
};
use crate::serve::{err_response, json_response, DaemonState};
use crate::serve_git::session_repo_root;

const OFFICIAL_NAME: &str = "official";
const OFFICIAL_SOURCE: &str = "anthropics/skills";

pub(crate) fn routes(base: &str) -> Router<Arc<DaemonState>> {
    Router::new()
        .route(&format!("{base}/api/skills"), get(skills_page))
        .route(&format!("{base}/api/skills/state"), get(state_page))
        .route(&format!("{base}/api/skills/available"), get(available_page))
        .route(&format!("{base}/api/skills/install"), post(install))
        .route(&format!("{base}/api/skills/remove"), post(remove))
        .route(&format!("{base}/api/skills/marketplaces"), post(add_source))
        .route(
            &format!("{base}/api/skills/marketplaces/remove"),
            post(remove_source),
        )
        .route(&format!("{base}/api/skills/sync"), post(sync))
        .route(&format!("{base}/api/skills/invoke"), post(invoke))
}

#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum SkillScope {
    User,
    Project,
}

type Fail = (StatusCode, String);

fn fail(status: StatusCode, message: impl Into<String>) -> Fail {
    (status, message.into())
}

fn reply<T: Serialize>(result: Result<T, Fail>) -> Response {
    match result {
        Ok(value) => json_response(&value),
        Err((status, message)) => err_response(status, &message),
    }
}

async fn project_root(state: &DaemonState, session: Option<&str>) -> Result<PathBuf, Fail> {
    match session.filter(|s| !s.is_empty()) {
        Some(session) => session_repo_root(state, session).await,
        None => Err(fail(
            StatusCode::BAD_REQUEST,
            "project scope needs a session to find the project",
        )),
    }
}

struct Locations {
    skills_dir: PathBuf,
    lock_path: PathBuf,
}

async fn locations(
    state: &DaemonState,
    scope: SkillScope,
    session: Option<&str>,
) -> Result<Locations, Fail> {
    match scope {
        SkillScope::User => Ok(Locations {
            skills_dir: skills_dir().map_err(internal)?,
            lock_path: installed_path().map_err(internal)?,
        }),
        SkillScope::Project => {
            let root = project_root(state, session).await?;
            Ok(Locations {
                skills_dir: root.join(".forge/skills"),
                lock_path: root.join(".forge/installed-skills.toml"),
            })
        }
    }
}

fn internal(error: anyhow::Error) -> Fail {
    fail(StatusCode::INTERNAL_SERVER_ERROR, format!("{error:#}"))
}

fn registry_with_official() -> Result<Marketplaces, Fail> {
    let mut registry =
        load_marketplaces_at(&marketplaces_path().map_err(internal)?).map_err(internal)?;
    registry
        .marketplaces
        .entry(OFFICIAL_NAME.to_string())
        .or_insert(MarketplaceEntry {
            source: OFFICIAL_SOURCE.to_string(),
            git_ref: None,
        });
    Ok(registry)
}

#[derive(Serialize)]
struct SkillRow {
    name: String,
    description: String,
    scope: String,
    tier: Option<String>,
    resources: usize,
    uses: u64,
    pack: Option<String>,
}

#[derive(Deserialize)]
struct SessionQuery {
    session: Option<String>,
}

async fn skills_page(
    State(state): State<Arc<DaemonState>>,
    Query(q): Query<SessionQuery>,
) -> Response {
    let project = match q.session.as_deref() {
        Some(session) => session_repo_root(&state, session).await.ok(),
        None => None,
    };
    let rows = tokio::task::spawn_blocking(move || {
        let sources = match &project {
            Some(root) => forge_config::command_sources_in(root),
            None => forge_config::command_sources(),
        };
        let catalog = forge_skills::Catalog::load(&sources);
        let uses = forge_config::skill_usage::skill_use_counts();
        let user_lock = installed_path()
            .ok()
            .and_then(|p| load_installed_at(&p).ok());
        let project_lock = project
            .as_ref()
            .and_then(|r| load_installed_at(&r.join(".forge/installed-skills.toml")).ok());
        let pack_of = |scope: &str, name: &str| {
            let lock = if scope == "project" {
                project_lock.as_ref()
            } else {
                user_lock.as_ref()
            }?;
            lock.skills
                .iter()
                .find(|(_, entry)| entry.files.iter().any(|f| f == name))
                .map(|(pack, _)| pack.clone())
        };
        let mut skills: Vec<SkillRow> = catalog
            .all_skills()
            .into_iter()
            .map(|skill| {
                let scope = skill.scope.label().to_string();
                SkillRow {
                    pack: pack_of(&scope, &skill.name),
                    uses: uses.get(&skill.name).copied().unwrap_or(0),
                    name: skill.name.clone(),
                    description: skill.description.clone(),
                    scope,
                    tier: skill
                        .tier
                        .map(|tier| format!("{tier:?}").to_ascii_lowercase()),
                    resources: skill.resources.len(),
                }
            })
            .collect();
        skills.sort_by(|a, b| a.name.cmp(&b.name));
        skills
    })
    .await;
    match rows {
        Ok(rows) => json_response(&rows),
        Err(_) => err_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not read skills catalog",
        ),
    }
}

#[derive(Serialize)]
struct SourceRow {
    name: String,
    source: String,
    #[serde(rename = "ref")]
    git_ref: Option<String>,
    official: bool,
}

#[derive(Serialize)]
struct PackRow {
    name: String,
    source: String,
    marketplace: Option<String>,
    scope: &'static str,
    skills: Vec<String>,
}

#[derive(Serialize)]
struct StatePage {
    sources: Vec<SourceRow>,
    packs: Vec<PackRow>,
    sync_targets: Vec<&'static str>,
}

async fn state_page(
    State(state): State<Arc<DaemonState>>,
    Query(q): Query<SessionQuery>,
) -> Response {
    reply(build_state(&state, q.session.as_deref()).await)
}

async fn build_state(state: &DaemonState, session: Option<&str>) -> Result<StatePage, Fail> {
    let registry = registry_with_official()?;
    let sources = registry
        .marketplaces
        .iter()
        .map(|(name, entry)| SourceRow {
            official: name == OFFICIAL_NAME && entry.source == OFFICIAL_SOURCE,
            name: name.clone(),
            source: entry.source.clone(),
            git_ref: entry.git_ref.clone(),
        })
        .collect();
    let mut packs = Vec::new();
    let mut collect = |scope: &'static str, lock_path: &Path| {
        if let Ok(lock) = load_installed_at(lock_path) {
            for (name, entry) in lock.skills {
                packs.push(PackRow {
                    name,
                    source: entry.source,
                    marketplace: entry.marketplace,
                    scope,
                    skills: entry.files,
                });
            }
        }
    };
    collect("user", &installed_path().map_err(internal)?);
    if let Ok(loc) = locations(state, SkillScope::Project, session).await {
        collect("project", &loc.lock_path);
    }
    Ok(StatePage {
        sources,
        packs,
        sync_targets: vec![".agents/skills", ".claude/skills"],
    })
}

#[derive(Deserialize)]
struct AvailableQuery {
    marketplace: String,
}

async fn available_page(Query(q): Query<AvailableQuery>) -> Response {
    reply(
        async {
            let registry = registry_with_official()?;
            let entry = registry
                .marketplaces
                .get(&q.marketplace)
                .ok_or_else(|| fail(StatusCode::NOT_FOUND, "no such marketplace"))?;
            list_available(&entry.source, entry.git_ref.as_deref())
                .await
                .map_err(|e| fail(StatusCode::BAD_GATEWAY, format!("{e:#}")))
        }
        .await,
    )
}

#[derive(Deserialize)]
struct InstallBody {
    source: String,
    scope: SkillScope,
    marketplace: Option<String>,
    session: Option<String>,
}

#[derive(Serialize)]
struct InstallReply {
    name: String,
    installed: Vec<String>,
    skipped: Vec<String>,
    ignored_components: Vec<String>,
}

async fn install(State(state): State<Arc<DaemonState>>, Json(body): Json<InstallBody>) -> Response {
    reply(
        async {
            let source = body.source.trim();
            if source.is_empty() {
                return Err(fail(StatusCode::BAD_REQUEST, "name a skill or repository"));
            }
            let loc = locations(&state, body.scope, body.session.as_deref()).await?;
            let registry = registry_with_official()?;
            let report = install_pack_to(
                source,
                body.marketplace.as_deref().filter(|m| !m.is_empty()),
                &registry,
                &loc.skills_dir,
                &loc.lock_path,
            )
            .await
            .map_err(|e| fail(StatusCode::BAD_REQUEST, format!("{e:#}")))?;
            Ok(InstallReply {
                name: report.name,
                installed: report.installed,
                skipped: report.skipped,
                ignored_components: report.ignored_components,
            })
        }
        .await,
    )
}

#[derive(Deserialize)]
struct RemoveBody {
    name: String,
    scope: SkillScope,
    session: Option<String>,
}

async fn remove(State(state): State<Arc<DaemonState>>, Json(body): Json<RemoveBody>) -> Response {
    reply(
        async {
            let loc = locations(&state, body.scope, body.session.as_deref()).await?;
            let removed = remove_plugin_at(&loc.skills_dir, &loc.lock_path, &body.name)
                .map_err(|e| fail(StatusCode::BAD_REQUEST, format!("{e:#}")))?;
            if !removed {
                return Err(fail(
                    StatusCode::NOT_FOUND,
                    "nothing to remove by that name",
                ));
            }
            Ok(serde_json::json!({ "removed": body.name }))
        }
        .await,
    )
}

#[derive(Deserialize)]
struct SourceBody {
    name: String,
    source: Option<String>,
    #[serde(rename = "ref")]
    git_ref: Option<String>,
}

async fn add_source(Json(body): Json<SourceBody>) -> Response {
    reply(
        async {
            let name = body.name.trim();
            let source = body.source.as_deref().unwrap_or("").trim();
            if name.is_empty() || source.is_empty() || name == OFFICIAL_NAME {
                return Err(fail(
                    StatusCode::BAD_REQUEST,
                    "a marketplace needs its own name and a git source",
                ));
            }
            let path = marketplaces_path().map_err(internal)?;
            add_marketplace_at(&path, name, source, body.git_ref.filter(|r| !r.is_empty()))
                .map_err(internal)?;
            Ok(serde_json::json!({ "added": name }))
        }
        .await,
    )
}

async fn remove_source(Json(body): Json<SourceBody>) -> Response {
    reply(
        async {
            let path = marketplaces_path().map_err(internal)?;
            let removed = remove_marketplace_at(&path, body.name.trim()).map_err(internal)?;
            Ok(serde_json::json!({ "removed": removed }))
        }
        .await,
    )
}

#[derive(Deserialize)]
struct SyncBody {
    scope: SkillScope,
    session: Option<String>,
}

#[derive(Serialize, Default, Debug, PartialEq, Eq)]
pub(crate) struct SyncReport {
    pub(crate) linked: Vec<String>,
    pub(crate) conflicts: Vec<String>,
    pub(crate) targets: Vec<String>,
}

async fn sync(State(state): State<Arc<DaemonState>>, Json(body): Json<SyncBody>) -> Response {
    reply(
        async {
            let (base, sources) = match body.scope {
                SkillScope::Project => {
                    let root = project_root(&state, body.session.as_deref()).await?;
                    let sources = forge_config::command_sources_in(&root);
                    (root, sources)
                }
                SkillScope::User => {
                    let home = forge_config::home_dir().ok_or_else(|| {
                        fail(StatusCode::INTERNAL_SERVER_ERROR, "no home directory")
                    })?;
                    (home, forge_config::command_sources())
                }
            };
            let only_user = body.scope == SkillScope::User;
            tokio::task::spawn_blocking(move || sync_skills(&sources, &base, only_user))
                .await
                .map_err(|_| fail(StatusCode::INTERNAL_SERVER_ERROR, "sync failed"))
        }
        .await,
    )
}

/// Link every on-disk skill of `sources` into `<base>/.agents/skills` and `<base>/.claude/skills`.
/// A real directory already there is never touched; it is reported as a conflict.
pub(crate) fn sync_skills(
    sources: &forge_skills::Sources,
    base: &Path,
    only_user: bool,
) -> SyncReport {
    let catalog = forge_skills::Catalog::load(sources);
    let mut report = SyncReport::default();
    let targets = [base.join(".agents/skills"), base.join(".claude/skills")];
    for target in &targets {
        report.targets.push(target.display().to_string());
    }
    for skill in catalog.all_skills() {
        let wanted = match skill.scope {
            forge_skills::Scope::User => true,
            forge_skills::Scope::Project => !only_user,
            _ => false,
        };
        if !wanted || skill.body.is_some() || !skill.dir.is_dir() {
            continue;
        }
        for target in &targets {
            let link = target.join(&skill.name);
            if std::fs::create_dir_all(target).is_err() {
                continue;
            }
            match std::fs::symlink_metadata(&link) {
                Ok(meta) if meta.file_type().is_symlink() => {
                    if std::fs::read_link(&link).ok().as_deref() == Some(skill.dir.as_path()) {
                        report.linked.push(link.display().to_string());
                        continue;
                    }
                    let _ = std::fs::remove_file(&link);
                }
                Ok(_) => {
                    report.conflicts.push(link.display().to_string());
                    continue;
                }
                Err(_) => {}
            }
            if link_dir(&skill.dir, &link).is_ok() {
                report.linked.push(link.display().to_string());
            }
        }
    }
    report
}

#[cfg(unix)]
fn link_dir(from: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(from, link)
}

#[cfg(not(unix))]
fn link_dir(from: &Path, link: &Path) -> std::io::Result<()> {
    crate::cli::commands::import::copy_dir(from, link)
}

#[derive(Deserialize)]
struct InvokeBody {
    name: String,
    task: Option<String>,
    session: Option<String>,
}

#[derive(Serialize)]
struct InvokeReply {
    name: String,
    prompt: String,
    uses: u64,
}

async fn invoke(State(state): State<Arc<DaemonState>>, Json(body): Json<InvokeBody>) -> Response {
    reply(
        async {
            let project = match body.session.as_deref() {
                Some(session) => session_repo_root(&state, session).await.ok(),
                None => None,
            };
            let name = body.name.trim().to_string();
            let task = body.task.unwrap_or_default().trim().to_string();
            tokio::task::spawn_blocking(move || {
                let sources = match &project {
                    Some(root) => forge_config::command_sources_in(root),
                    None => forge_config::command_sources(),
                };
                let catalog = forge_skills::Catalog::load(&sources);
                let known = catalog.all_skills().into_iter().any(|s| s.name == name);
                if !known {
                    return Err(fail(
                        StatusCode::NOT_FOUND,
                        format!("no skill named {name}"),
                    ));
                }
                forge_config::skill_usage::record_skill_use(&name);
                let uses = forge_config::skill_usage::skill_use_counts()
                    .get(&name)
                    .copied()
                    .unwrap_or(0);
                let prompt = if task.is_empty() {
                    format!("/skill {name}")
                } else {
                    format!("/skill {name} {task}")
                };
                Ok(InvokeReply { name, prompt, uses })
            })
            .await
            .map_err(|_| fail(StatusCode::INTERNAL_SERVER_ERROR, "invoke failed"))?
        }
        .await,
    )
}

#[cfg(test)]
#[path = "serve_skills_tests.rs"]
mod tests;
