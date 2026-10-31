//! MCP servers switched off for one session (`POST /api/sessions` with `mcp_off`).
//!
//! Carried as a task-local around the session build instead of through every builder signature:
//! the build reads its MCP catalog inside the awaited future, so the scope reaches it.

use std::future::Future;
use std::path::{Path, PathBuf};

tokio::task_local! {
    static MCP_OFF: Vec<String>;
}

pub(crate) async fn scoped<F: Future>(off: Vec<String>, build: F) -> F::Output {
    MCP_OFF.scope(off, build).await
}

pub(crate) fn apply(mcp: &mut forge_config::McpConfig) {
    let _ = MCP_OFF.try_with(|off| mcp.servers.retain(|server| !off.contains(&server.name)));
}

pub(crate) fn current() -> Vec<String> {
    MCP_OFF.try_with(Clone::clone).unwrap_or_default()
}

/// Sidecar next to the store, so the daemon and the `forge mcp-serve` child it spawns for a bridge
/// turn (which share `FORGE_DB`) read one record without a schema migration.
fn sidecar(db: &Path, session_id: &str) -> Option<PathBuf> {
    let safe = !session_id.is_empty()
        && session_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'));
    safe.then(|| {
        db.parent()
            .unwrap_or_else(|| Path::new("."))
            .join("session-mcp-off")
            .join(format!("{session_id}.json"))
    })
}

pub(crate) fn load(db: Option<&Path>, session_id: &str) -> Vec<String> {
    db.and_then(|db| sidecar(db, session_id))
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub(crate) fn save(db: Option<&Path>, session_id: &str, off: &[String]) {
    let Some(path) = db.and_then(|db| sidecar(db, session_id)) else {
        return;
    };
    if off.is_empty() {
        let _ = std::fs::remove_file(path);
        return;
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string(off) {
        let _ = std::fs::write(path, text);
    }
}

/// Servers to leave out of a session being built: the explicit request if there is one, otherwise
/// what the session being resumed or forked from had switched off.
pub(crate) fn inherited(resume: Option<&str>) -> Vec<String> {
    let explicit = current();
    if !explicit.is_empty() {
        return explicit;
    }
    let Some(resume) = resume else {
        return Vec::new();
    };
    let Ok(store) = crate::cli::commands::replay::open_store() else {
        return Vec::new();
    };
    let id = crate::cli::commands::replay::resolve_session(&store, resume).unwrap_or_default();
    load(store.db_path(), &id)
}

/// Drop the servers the owning bridge session switched off. The `forge mcp-serve` child builds its
/// own MCP manager from the config file, so it reads the same sidecar by session id.
pub(crate) fn apply_persisted(
    mcp: &mut forge_config::McpConfig,
    db: Option<&Path>,
    session_id: &str,
) {
    let off = load(db, session_id);
    mcp.servers.retain(|server| !off.contains(&server.name));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(name: &str) -> forge_config::McpServerConfig {
        toml::from_str(&format!(
            "name = \"{name}\"\n[transport]\ntype = \"stdio\"\ncommand = \"x\"\n"
        ))
        .unwrap()
    }

    #[tokio::test]
    async fn only_the_scoped_build_drops_the_named_servers() {
        let config = forge_config::McpConfig {
            servers: vec![server("keep"), server("drop")],
            ..Default::default()
        };
        let mut outside = config.clone();
        apply(&mut outside);
        assert_eq!(outside.servers.len(), 2);
        let inside = scoped(vec!["drop".into()], async {
            let mut c = config.clone();
            apply(&mut c);
            c
        })
        .await;
        assert_eq!(inside.servers.len(), 1);
        assert_eq!(inside.servers[0].name, "keep");
    }

    #[test]
    fn the_exclusion_outlives_the_session_build_for_the_bridge_child_and_restores() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("forge.db");
        save(Some(&db), "sess-1", &["drop".into()]);
        let mut config = forge_config::McpConfig {
            servers: vec![server("keep"), server("drop")],
            ..Default::default()
        };
        apply_persisted(&mut config, Some(&db), "sess-1");
        assert_eq!(config.servers.len(), 1);
        assert_eq!(config.servers[0].name, "keep");
        let mut other = forge_config::McpConfig {
            servers: vec![server("keep"), server("drop")],
            ..Default::default()
        };
        apply_persisted(&mut other, Some(&db), "sess-2");
        assert_eq!(other.servers.len(), 2);
        assert!(sidecar(&db, "../escape").is_none());
        save(Some(&db), "sess-1", &[]);
        assert!(load(Some(&db), "sess-1").is_empty());
    }
}
