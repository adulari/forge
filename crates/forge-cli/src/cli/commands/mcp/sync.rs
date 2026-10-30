//! `forge mcp sync --to claude,codex`.

use anyhow::{bail, Result};
use forge_config::mcp_sync::{self, SyncTarget};

use crate::McpSyncTarget;

pub(crate) fn mcp_sync(targets: &[McpSyncTarget], dry_run: bool) -> Result<()> {
    let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) else {
        bail!("HOME is not set");
    };
    let config = super::super::load_config()?;
    let servers: Vec<_> = config
        .mcp
        .active_servers()
        .filter(|server| server.name != "forge")
        .collect();
    if servers.is_empty() {
        println!("no enabled MCP servers to sync");
        return Ok(());
    }
    let mut seen = Vec::new();
    for target in targets {
        let target = match target {
            McpSyncTarget::Claude => SyncTarget::Claude,
            McpSyncTarget::Codex => SyncTarget::Codex,
        };
        if seen.contains(&target.label()) {
            continue;
        }
        seen.push(target.label());
        let path = target.path(&home);
        let report = match target {
            SyncTarget::Claude => mcp_sync::sync_claude(&path, &servers, dry_run),
            SyncTarget::Codex => mcp_sync::sync_codex(&path, &servers, dry_run),
        }
        .map_err(|error| anyhow::anyhow!("{}: {error}", target.label()))?;
        let verb = if dry_run { "would write" } else { "wrote" };
        println!(
            "{}: {verb} {} ({} added, {} updated, {} unchanged)",
            target.label(),
            path.display(),
            report.added.len(),
            report.updated.len(),
            report.unchanged.len()
        );
    }
    Ok(())
}
