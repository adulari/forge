//! The CLI side of `/worktree`, `/project`, `/clone` and `/home`: opens the overlay, feeds it
//! workspace snapshots and performs what a [`WsAction`] asks for. Git and network work runs on
//! blocking tasks that post [`WsEvent`]s, so the render loop never waits on it. A worktree switch
//! goes through [`forge_core::Session::run_worktree_request`], the exact path the model's
//! `worktree` tool takes, so the workspace root, Lattice and cwd move the same way.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use forge_core::project_ops::{self, PullStatus};
use forge_core::project_registry::{ProjectPatch, ProjectRegistry};
use forge_core::worktree_ops::{self, Request};
use forge_core::{remote_url, worktree_setup, Session};
use forge_tui::workspace_menu::{Kind, RepoRow, WorkspaceData, WsAction, WsEvent, WtRow};
use forge_tui::{App, Deferred, KeyKind};
use serde_json::json;

type SharedSession = Arc<tokio::sync::Mutex<Session>>;

/// What `/worktree <args>` asks for.
#[derive(Debug, PartialEq)]
pub(crate) enum WtCommand {
    Open,
    Pull,
    Run(serde_json::Value),
}

const USAGE: &str =
    "usage: /worktree [new [name] [--carry] | list | enter <name|path> | exit | remove <name> | pull]";

pub(crate) fn parse_worktree_args(arg: &str) -> Result<WtCommand, String> {
    let mut words = arg.split_whitespace();
    let Some(verb) = words.next() else {
        return Ok(WtCommand::Open);
    };
    let rest: Vec<&str> = words.collect();
    let one_target = |rest: &[&str]| -> Result<String, String> {
        match rest {
            [target] => Ok((*target).to_string()),
            _ => Err(USAGE.to_string()),
        }
    };
    match verb.to_lowercase().as_str() {
        "list" | "ls" if rest.is_empty() => Ok(WtCommand::Run(json!({"action": "list"}))),
        "exit" | "leave" if rest.is_empty() => Ok(WtCommand::Run(json!({"action": "exit"}))),
        "pull" if rest.is_empty() => Ok(WtCommand::Pull),
        "enter" | "switch" => Ok(WtCommand::Run(
            json!({"action": "enter", "path": one_target(&rest)?}),
        )),
        "remove" | "rm" | "delete" => Ok(WtCommand::Run(
            json!({"action": "remove", "path": one_target(&rest)?}),
        )),
        "new" | "create" | "add" => {
            let carry = rest.contains(&"--carry");
            if let Some(flag) = rest.iter().find(|w| w.starts_with('-') && **w != "--carry") {
                return Err(format!("unknown option {flag}. {USAGE}"));
            }
            let names: Vec<&&str> = rest.iter().filter(|w| !w.starts_with('-')).collect();
            if names.len() > 1 {
                return Err(USAGE.to_string());
            }
            let mut args = json!({"action": "create"});
            if let Some(name) = names.first() {
                forge_tui::workspace_menu::valid_worktree_name(name)?;
                args["name"] = json!(name);
            }
            if carry {
                args["dirty"] = json!("carry");
            }
            Ok(WtCommand::Run(args))
        }
        _ => Err(USAGE.to_string()),
    }
}

fn root_of(app: &App) -> PathBuf {
    app.workspace_path()
        .map(Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

fn git_ok(dir: &Path, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn is_dirty(dir: &Path) -> bool {
    git_ok(dir, &["status", "--porcelain"]).is_some_and(|s| !s.is_empty())
}

fn clone_parent() -> PathBuf {
    std::env::var_os("HOME")
        .map(|h| PathBuf::from(h).join("Projects"))
        .unwrap_or_else(|| PathBuf::from("Projects"))
}

fn tilde(path: &Path) -> String {
    let text = path.display().to_string();
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && text.starts_with(&home) => {
            format!("~{}", &text[home.len()..])
        }
        _ => text,
    }
}

/// Everything the overlay shows, read off the render thread.
fn load_data(root: &Path) -> WorkspaceData {
    let top = git_ok(root, &["rev-parse", "--show-toplevel"]).map(|t| {
        let path = PathBuf::from(t);
        path.canonicalize().unwrap_or(path)
    });
    let mut data = WorkspaceData {
        root: tilde(root),
        is_git: top.is_some(),
        is_home: project_ops::home_dir().is_ok_and(|h| {
            h.canonicalize().unwrap_or(h)
                == root.canonicalize().unwrap_or_else(|_| root.to_path_buf())
        }),
        suggested_name: worktree_setup::friendly_name(),
        clone_parent: tilde(&clone_parent()),
        ..WorkspaceData::default()
    };
    let mut project_path = root.to_path_buf();
    if let Some(top) = &top {
        data.dirty = is_dirty(top);
        match worktree_ops::list(top) {
            Ok(entries) => {
                if let Some(main) = entries.iter().find(|e| e.main) {
                    project_path = main.path.clone();
                }
                data.worktrees = entries
                    .iter()
                    .map(|e| WtRow {
                        branch: e
                            .branch
                            .clone()
                            .unwrap_or_else(|| format!("detached @{}", e.head)),
                        path: e.path.display().to_string(),
                        display: tilde(&e.path),
                        dirty: if &e.path == top {
                            data.dirty
                        } else {
                            is_dirty(&e.path)
                        },
                        current: &e.path == top,
                        main: e.main,
                        merged: e.branch.as_deref().is_none_or(|b| {
                            git_ok(&project_path, &["merge-base", "--is-ancestor", b, "HEAD"])
                                .is_some()
                        }),
                    })
                    .collect();
            }
            Err(error) => data.error = Some(error),
        }
    }
    let registry = ProjectRegistry::load();
    let key = project_path.display().to_string();
    data.project_name = registry
        .entry(&key)
        .and_then(|e| e.name.clone())
        .or_else(|| {
            project_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
        })
        .unwrap_or(key);
    data
}

fn spawn_refresh(app: &App) {
    let inbox = app.workspace_menu.inbox();
    let root = root_of(app);
    tokio::task::spawn_blocking(move || inbox.post(WsEvent::Data(load_data(&root))));
}

pub(crate) fn open(app: &mut App, kind: Kind) {
    app.workspace_menu.open(kind);
    spawn_refresh(app);
}

/// Called every loop turn: load the first snapshot, apply posted events.
pub(crate) fn tick(app: &mut App) -> bool {
    if app.workspace_menu.take_refresh() {
        spawn_refresh(app);
    }
    app.workspace_menu.advance()
}

fn notes(text: &str) -> Deferred {
    Deferred::Notes(
        text.lines()
            .filter(|l| !l.starts_with("This session now works in"))
            .map(str::to_string)
            .collect(),
    )
}

fn spawn_request(session: &SharedSession, app: &mut App, args: serde_json::Value, label: &str) {
    let request = match Request::parse(&args) {
        Ok(r) => r,
        Err(error) => {
            app.note(&format!("⚠ {error}"));
            return;
        }
    };
    app.note(&format!("⧖ {label}…"));
    let (session, deferred, inbox) = (
        session.clone(),
        app.deferred_inbox(),
        app.workspace_menu.inbox(),
    );
    tokio::spawn(async move {
        let (text, ok) = session.lock().await.run_worktree_request(&request).await;
        let marked = if (ok && !text.contains("setup FAILED")) || text.starts_with('⚠') {
            text.clone()
        } else {
            format!("⚠ {text}")
        };
        deferred.post(notes(&marked));
        inbox.post(WsEvent::OpDone { ok, text });
    });
}

fn spawn_pull(session: &SharedSession, app: &mut App) {
    app.note("⧖ pulling (local changes are stashed and re-applied)…");
    let (session, deferred, inbox) = (
        session.clone(),
        app.deferred_inbox(),
        app.workspace_menu.inbox(),
    );
    tokio::spawn(async move {
        let root = session.lock().await.workspace_root().to_path_buf();
        let outcome = tokio::task::spawn_blocking(move || project_ops::pull(&root)).await;
        let Ok(outcome) = outcome else {
            inbox.post(WsEvent::Pulled {
                ok: false,
                message: "pull task failed".into(),
                conflicts: Vec::new(),
                handoff: None,
            });
            return;
        };
        let ok = matches!(outcome.status, PullStatus::UpToDate | PullStatus::Updated);
        let mut lines = vec![format!(
            "{} pull: {}",
            if ok { "✓" } else { "⚠" },
            outcome.message
        )];
        lines.extend(outcome.conflicts.iter().map(|f| format!("  conflict: {f}")));
        deferred.post(Deferred::Notes(lines));
        inbox.post(WsEvent::Pulled {
            ok,
            message: outcome.message,
            conflicts: outcome.conflicts,
            handoff: outcome.handoff_prompt,
        });
    });
}

fn spawn_clone(app: &mut App, url: String) {
    app.note(&format!("⧖ cloning {url}…"));
    let (deferred, inbox) = (app.deferred_inbox(), app.workspace_menu.inbox());
    tokio::task::spawn_blocking(move || {
        let parent = clone_parent();
        let result = std::fs::create_dir_all(&parent)
            .map_err(|e| format!("creating {}: {e}", parent.display()))
            .and_then(|()| project_ops::clone_repo(&url, &parent, None))
            .map(|p| p.display().to_string());
        deferred.post(Deferred::Notes(vec![match &result {
            Ok(path) => format!("✓ cloned to {path}"),
            Err(error) => format!("⚠ clone failed: {error}"),
        }]));
        inbox.post(WsEvent::Cloned(result));
    });
}

fn load_repos(app: &App) {
    let inbox = app.workspace_menu.inbox();
    tokio::task::spawn_blocking(move || {
        let gh = project_ops::gh_repos();
        inbox.post(WsEvent::Repos(if gh.available {
            Ok(gh
                .repos
                .into_iter()
                .map(|r| RepoRow {
                    name: r.name_with_owner,
                    url: r.url,
                    description: r.description,
                    private: r.private,
                })
                .collect())
        } else {
            Err(gh.note)
        }));
    });
}

/// Switching locks the session, which a running turn holds for its whole length. Doing that
/// inside the key loop froze the TUI until the turn ended, so the switch runs on its own task and
/// waits for the turn there.
fn switch_project(session: &SharedSession, app: &mut App, path: &Path, label: &str) {
    if session.try_lock().is_err() {
        app.note(&format!(
            "⧖ {label}: waiting for the running turn to finish before switching…"
        ));
    }
    let (session, deferred, inbox) = (
        session.clone(),
        app.deferred_inbox(),
        app.workspace_menu.inbox(),
    );
    let (path, label) = (path.to_path_buf(), label.to_string());
    tokio::spawn(async move {
        let result = session.lock().await.switch_project(&path);
        let (ok, text) = match result {
            Ok(()) => (
                true,
                format!("✓ {label}: now working in {}", path.display()),
            ),
            Err(error) => (
                false,
                format!("⚠ could not switch to {}: {error}", path.display()),
            ),
        };
        deferred.post(Deferred::Notes(vec![text.clone()]));
        inbox.post(WsEvent::OpDone { ok, text });
    });
}

fn go_home(session: &SharedSession, app: &mut App) {
    match project_ops::home_dir() {
        Ok(dir) => switch_project(session, app, &dir, "Home"),
        Err(error) => app.note(&format!("⚠ /home: {error}")),
    }
}

/// Mutate the project registry entry of the current project.
fn registry_patch(app: &App, patch: ProjectPatch) -> Result<(), String> {
    let root = root_of(app);
    let top = git_ok(&root, &["rev-parse", "--show-toplevel"]).map(PathBuf::from);
    let project = top
        .and_then(|t| worktree_ops::list(&t).ok())
        .and_then(|l| l.into_iter().find(|e| e.main).map(|e| e.path))
        .unwrap_or(root);
    let mut registry = ProjectRegistry::load();
    registry.apply(&project.display().to_string(), &patch)?;
    registry.save()
}

async fn perform(action: WsAction, session: &SharedSession, app: &mut App) -> Option<String> {
    match action {
        WsAction::None | WsAction::Close => {}
        WsAction::Refresh => {
            app.workspace_menu.screen = forge_tui::workspace_menu::Screen::List;
            spawn_refresh(app);
        }
        WsAction::LoadRepos => load_repos(app),
        WsAction::EnterWorktree { path } => {
            spawn_request(
                session,
                app,
                json!({"action": "enter", "path": path}),
                "switching",
            );
        }
        WsAction::CreateWorktree { name, carry } => {
            let mut args = json!({"action": "create", "name": name});
            if carry {
                args["dirty"] = json!("carry");
            }
            spawn_request(session, app, args, "creating worktree");
        }
        WsAction::RemoveWorktree {
            path,
            force,
            keep_branch,
            ..
        } => {
            let args = json!({
                "action": "remove",
                "path": path,
                "force": force,
                "keep_branch": keep_branch,
                "keep_unmerged": true,
            });
            spawn_request(session, app, args, "removing");
        }
        WsAction::Pull => spawn_pull(session, app),
        WsAction::GoHome => go_home(session, app),
        WsAction::InitHere => {
            let root = root_of(app);
            let result = tokio::task::spawn_blocking(move || project_ops::init_repo(&root)).await;
            let (ok, text) = match result {
                Ok(Ok(())) => (
                    true,
                    "✓ initialized a git repository with an empty first commit".to_string(),
                ),
                Ok(Err(error)) => (false, format!("git init: {error}")),
                Err(error) => (false, format!("git init failed: {error}")),
            };
            app.note(&if ok {
                text.clone()
            } else {
                format!("⚠ {text}")
            });
            app.workspace_menu
                .inbox()
                .post(WsEvent::OpDone { ok, text });
        }
        WsAction::Clone { url } => match remote_url::validate_remote_url(&url) {
            Ok(()) => {
                app.workspace_menu
                    .set_busy(format!("Cloning {url} into {}", tilde(&clone_parent())));
                spawn_clone(app, url);
            }
            Err(error) => app.workspace_menu.input_error(error),
        },
        WsAction::SwitchProject { path } => {
            switch_project(session, app, Path::new(&path), "project")
        }
        WsAction::RenameProject { name } => {
            let result = registry_patch(
                app,
                ProjectPatch {
                    name: Some(name.clone()),
                    ..ProjectPatch::default()
                },
            );
            let (ok, text) = match result {
                Ok(()) if name.is_empty() => (true, "✓ project name reset".to_string()),
                Ok(()) => (true, format!("✓ project renamed to {name}")),
                Err(error) => (false, error),
            };
            app.note(&if ok {
                text.clone()
            } else {
                format!("⚠ {text}")
            });
            app.workspace_menu
                .inbox()
                .post(WsEvent::OpDone { ok, text });
        }
        WsAction::HideProject => {
            let result = registry_patch(
                app,
                ProjectPatch {
                    disabled: Some(true),
                    ..ProjectPatch::default()
                },
            );
            let (ok, text) = match result {
                Ok(()) => (true, "✓ project hidden from project lists".to_string()),
                Err(error) => (false, error),
            };
            app.note(&if ok {
                text.clone()
            } else {
                format!("⚠ {text}")
            });
            app.workspace_menu
                .inbox()
                .post(WsEvent::OpDone { ok, text });
        }
        WsAction::Submit { prompt } => return Some(prompt),
    }
    None
}

/// One keystroke while the overlay is open. A handoff becomes the next prompt: it is typed into
/// the composer and submitted with a synthesized Enter, so it takes the ordinary prompt path.
pub(crate) async fn key(
    app: &mut App,
    session: &SharedSession,
    remote_keys: &mut std::collections::VecDeque<KeyKind>,
    key: KeyKind,
) {
    let action = app.workspace_menu.key(key);
    if let Some(prompt) = perform(action, session, app).await {
        app.input = prompt;
        app.input_cursor = app.input.len();
        remote_keys.push_back(KeyKind::Enter);
    }
}

/// `/worktree <args>`, `/worktree` with none opens the overlay.
pub(crate) fn worktree_command(
    arg: &str,
    session: &SharedSession,
    app: &mut App,
    interactive: bool,
) {
    match parse_worktree_args(arg) {
        Ok(WtCommand::Open) if interactive => open(app, Kind::Worktrees),
        Ok(WtCommand::Open) => spawn_request(session, app, json!({"action": "list"}), "listing"),
        Ok(WtCommand::Pull) => spawn_pull(session, app),
        Ok(WtCommand::Run(args)) => {
            let label = match args["action"].as_str() {
                Some("create") => "creating worktree",
                Some("enter") => "switching worktree",
                Some("exit") => "leaving worktree",
                Some("remove") => "removing worktree",
                _ => "listing worktrees",
            };
            spawn_request(session, app, args, label);
        }
        Err(error) => app.note(&format!("⚠ {error}")),
    }
}

/// `/clone <url>`: validate first so a refused URL is explained on the spot.
pub(crate) fn clone_command(url: &str, app: &mut App, interactive: bool) {
    let url = url.trim();
    if url.is_empty() {
        if interactive {
            open(app, Kind::Projects);
            app.workspace_menu.key(KeyKind::Char('c'));
            load_repos(app);
        } else {
            app.note("usage: /clone <url>");
        }
        return;
    }
    match remote_url::validate_remote_url(url) {
        Ok(()) => spawn_clone(app, url.to_string()),
        Err(error) => app.note(&format!("⚠ /clone: {error}")),
    }
}

pub(crate) async fn home_command(session: &SharedSession, app: &mut App) {
    go_home(session, app);
}

/// A one-time hint on a non-git workspace: shown once ever, never in the Home folder.
pub(crate) fn non_git_hint(app: &mut App, root: &Path) {
    if git_ok(root, &["rev-parse", "--show-toplevel"]).is_some() {
        return;
    }
    if project_ops::home_dir().is_ok_and(|h| h == root) {
        return;
    }
    let Some(marker) = forge_config::config_dir().map(|d| d.join("non-git-hint-shown")) else {
        return;
    };
    if marker.exists() {
        return;
    }
    app.note("not a git repository: /project to initialize");
    let _ = marker.parent().map(std::fs::create_dir_all);
    let _ = std::fs::write(marker, "");
}

#[cfg(test)]
#[path = "workspace_menu_tests.rs"]
mod tests;
