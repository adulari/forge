//! Everyday Claude Code commands: `/context`, `/diff`, `/permissions`, `/mcp reconnect`,
//! `/add-dir`, `/doctor` and `/rename`. Each answers from data the session already keeps; the
//! slow ones (`/diff`, `/doctor`, `/mcp reconnect`, `/rename`) run on their own task and post to
//! the chat's deferred inbox so the render loop never waits on git, a subprocess, a server connect
//! or a store write. `/diff`, `/doctor` and `/rename` also run while a turn holds the session lock,
//! so they never await it.

use std::sync::Arc;

use forge_core::Session;
use forge_tui::{App, CommandAction, Deferred};

mod context;
mod diff;
mod doctor;
mod permissions;

type SharedSession = Arc<tokio::sync::Mutex<Session>>;

pub(super) async fn dispatch(action: CommandAction, session: &SharedSession, app: &mut App) {
    match action {
        CommandAction::Context => {
            let report = session.lock().await.context_report();
            let lines = context::report_lines(&report, app.context_limit, app.context_tokens);
            app.note(&lines.join("\n"));
        }
        CommandAction::Diff(path) => {
            // The daemon's headless chat does not follow a git workspace; ask the session then.
            let workspace = app
                .workspace_path()
                .map(std::path::Path::to_path_buf)
                .or_else(|| {
                    let s = session.try_lock().ok()?;
                    Some(s.workspace_root().to_path_buf())
                });
            diff::start(app, workspace, path)
        }
        CommandAction::Permissions(arg) => permissions::run(session, app, &arg).await,
        CommandAction::McpReconnect(server) => reconnect_mcp(session, app, server).await,
        CommandAction::AddDir(path) => add_dir(session, app, &path).await,
        CommandAction::Doctor => doctor::start(app),
        CommandAction::Bug(text) => crate::reporting::bug_card(app, &text),
        CommandAction::Rename(title) => rename(session, app, &title).await,
        // `dispatch_command` routes only the actions above here.
        _ => {}
    }
}

/// `/mcp reconnect [server]`: drop and re-establish connections off the render loop. A server
/// can take its whole connect timeout to answer, and the session lock must stay free meanwhile.
async fn reconnect_mcp(session: &SharedSession, app: &mut App, server: Option<String>) {
    let Some(manager) = session.lock().await.mcp_manager() else {
        app.note("no MCP servers are configured — `forge mcp add` or `forge mcp import` adds one");
        return;
    };
    let targets = match server {
        Some(name) => vec![name],
        None => manager.configured_servers(),
    };
    if targets.is_empty() {
        app.note("no enabled MCP servers to reconnect");
        return;
    }
    app.note(&format!("⧖ reconnecting {}…", targets.join(", ")));
    let inbox = app.deferred_inbox();
    tokio::spawn(async move {
        let mut lines = Vec::new();
        for name in targets {
            lines.push(match manager.reconnect_server(&name).await {
                Ok(()) => format!("✓ MCP server '{name}' reconnected"),
                Err(reason) => format!("⚠ MCP server '{name}' did not reconnect: {reason}"),
            });
        }
        inbox.post(Deferred::Notes(lines));
    });
}

/// `/add-dir <path>`: the file tools may work there for the rest of the session.
async fn add_dir(session: &SharedSession, app: &mut App, path: &str) {
    let path = path.trim();
    if path.is_empty() {
        let s = session.lock().await;
        let dirs = s.added_dirs();
        if dirs.is_empty() {
            app.note("usage: /add-dir <path> — let the file tools work in another directory this session");
        } else {
            app.note(&format!("extra directories: {}", dirs.join(", ")));
        }
        return;
    }
    let expanded = expand_home(path);
    match session.lock().await.add_dir(&expanded) {
        Ok(dir) => app.note(&format!(
            "✓ added {} for this session — file tools treat it like the workspace (persist it with [tools] extra_roots)",
            dir.display()
        )),
        Err(e) => app.note(&format!("⚠ /add-dir: {e}")),
    }
}

fn expand_home(path: &str) -> std::path::PathBuf {
    match (path.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => std::path::Path::new(&home).join(rest),
        _ if path == "~" => std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| path.into()),
        _ => path.into(),
    }
}

/// `/rename <title>`: the name `/sessions`, the resume picker and the apps show. It touches the
/// session store and nothing else, so it never waits for a running turn: while one holds the
/// session lock the store comes from the session's detached asker, and the write runs on a
/// blocking thread (the store retries a busy database by sleeping) that posts its result back.
async fn rename(session: &SharedSession, app: &mut App, title: &str) {
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    let Some((store, id)) = session_store(session, &app.session_id) else {
        app.note("/rename is not ready for this turn yet; try again in a moment");
        return;
    };
    let inbox = app.deferred_inbox();
    tokio::task::spawn_blocking(move || {
        let note = if title.is_empty() {
            match store.session_title(&id) {
                Ok(Some(current)) => {
                    format!("this session is named “{current}” — /rename <title> to change it")
                }
                _ => "usage: /rename <title>".to_string(),
            }
        } else {
            match store.set_session_title(&id, &title) {
                Ok(()) => format!("✓ session renamed to “{title}”"),
                Err(e) => format!("⚠ /rename: {e}"),
            }
        };
        inbox.post(Deferred::Notes(vec![note]));
    });
}

/// The session's store and id without waiting for the session lock.
fn session_store(
    session: &SharedSession,
    live_id: &str,
) -> Option<(Arc<forge_store::Store>, String)> {
    if let Ok(s) = session.try_lock() {
        return Some((s.store.clone(), s.session_id().to_string()));
    }
    let asker = forge_core::busy_asker(live_id)?;
    Some((asker.store().clone(), asker.session_id().to_string()))
}

#[cfg(test)]
mod tests {
    use super::expand_home;

    #[test]
    fn add_dir_expands_a_leading_tilde_only() {
        let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
        if let Some(home) = home {
            assert_eq!(expand_home("~/src"), home.join("src"));
            assert_eq!(expand_home("~"), home);
        }
        assert_eq!(expand_home("../lib"), std::path::PathBuf::from("../lib"));
        assert_eq!(
            expand_home("/opt/x~y"),
            std::path::PathBuf::from("/opt/x~y")
        );
    }
}
