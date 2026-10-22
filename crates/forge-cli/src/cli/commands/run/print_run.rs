//! What a `forge run` invocation applies to its session once built, and around its turns:
//! `--effort`, `--fallback-model`, `--name`, `--max-budget-usd`, `--session-id`,
//! `--no-session-persistence`, plus the `system/init` metadata for stream-json.

use anyhow::{Context, Result};
use forge_core::Session;
use forge_store::Store;

use crate::cli::args::claude_compat::ClaudeCompat;
use crate::cli::args::SessionFlags;

#[derive(Debug, Default)]
pub(crate) struct RunSetup {
    effort: Option<forge_types::EffortLevel>,
    fallback_models: Vec<String>,
    name: Option<String>,
    max_budget_usd: Option<f64>,
    no_persistence: bool,
    /// The session's lifetime spend when this run began; the budget is this run's spend only.
    spend_at_start: f64,
}

impl RunSetup {
    pub(crate) fn from_flags(print: &ClaudeCompat, session: &SessionFlags) -> Self {
        Self {
            effort: session.effort.map(Into::into),
            fallback_models: session
                .fallback_model
                .as_deref()
                .map(super::pin_members)
                .unwrap_or_default(),
            name: session
                .name
                .as_deref()
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .map(str::to_string),
            max_budget_usd: print.max_budget_usd,
            no_persistence: print.no_session_persistence,
            spend_at_start: 0.0,
        }
    }

    /// Apply the session-level flags to a freshly built (or resumed) session.
    pub(crate) fn apply(&mut self, session: &mut Session) -> Result<()> {
        if let Some(effort) = self.effort {
            session.set_effort(Some(effort));
        }
        if !self.fallback_models.is_empty() {
            super::ensure_pin_providers_known(&self.fallback_models.join(","))?;
            session.set_fallback_models(self.fallback_models.clone());
        }
        if let Some(name) = &self.name {
            session
                .store
                .set_session_title(session.session_id(), name)
                .context("naming the session")?;
        }
        self.spend_at_start = spent(session);
        Ok(())
    }

    /// Arm `--max-budget-usd` for the next turn with what this run has left.
    pub(crate) fn arm_budget(&self, session: &mut Session) {
        if let Some(max) = self.max_budget_usd {
            let left = (max - (spent(session) - self.spend_at_start)).max(0.0);
            session.set_run_budget(Some(forge_core::RunBudget {
                usd_remaining: Some(left),
                ..Default::default()
            }));
        }
    }

    /// `--no-session-persistence`: the run is over, forget its conversation (spend is kept).
    pub(crate) fn finish(&self, session: &Session) {
        if self.no_persistence {
            if let Err(error) = session
                .store
                .forget_session_keeping_spend(session.session_id())
            {
                eprintln!("warning: could not delete the unsaved session: {error}");
            }
        }
    }
}

fn spent(session: &Session) -> f64 {
    session
        .store
        .session_cost(session.session_id())
        .unwrap_or(0.0)
}

/// `--session-id`: must be a UUID nobody uses yet. The row is created up front (with the mode the
/// run starts in) and the run then resumes it, so the id is the session's from its first event.
pub(crate) fn create_session_with_id(
    store: &Store,
    id: &str,
    mode: Option<forge_types::PermissionMode>,
) -> Result<String> {
    let id = id.trim().to_ascii_lowercase();
    anyhow::ensure!(
        is_uuid(&id),
        "--session-id must be a UUID (like 550e8400-e29b-41d4-a716-446655440000), got `{id}`"
    );
    anyhow::ensure!(
        !store.session_exists(&id)?,
        "session id {id} is already in use — resume it with `--resume {id}`"
    );
    let cwd = std::env::current_dir()?
        .canonicalize()
        .context("resolving the workspace")?;
    let mode = match mode {
        Some(mode) => mode,
        None => forge_config::load()
            .map(|c| c.permission_mode)
            .unwrap_or_default(),
    };
    store.ensure_session(&id, &cwd.display().to_string(), &format!("{mode:?}"))?;
    Ok(id)
}

fn is_uuid(id: &str) -> bool {
    let groups: Vec<&str> = id.split('-').collect();
    groups.len() == 5
        && groups
            .iter()
            .zip([8, 4, 4, 4, 12])
            .all(|(g, len)| g.len() == len && g.chars().all(|c| c.is_ascii_hexdigit()))
}

/// Claude Code's name for a permission mode, as `system/init` reports it.
fn cc_permission_mode(mode: forge_types::PermissionMode) -> &'static str {
    use forge_types::PermissionMode;
    match mode {
        PermissionMode::Default => "default",
        PermissionMode::AcceptEdits => "acceptEdits",
        PermissionMode::Bypass => "bypassPermissions",
        PermissionMode::Plan => "plan",
        PermissionMode::Auto => "auto",
    }
}

/// The session's metadata for stream-json's `system/init`.
pub(crate) fn stream_init(session: &Session) -> forge_tui::StreamInit {
    let catalog =
        forge_skills::Catalog::load(&forge_config::command_sources_in(session.workspace_root()));
    forge_tui::StreamInit {
        cwd: session.workspace_root().display().to_string(),
        model: session
            .pinned_model()
            .map(|set| set.join(","))
            .unwrap_or_else(|| "auto".to_string()),
        permission_mode: cc_permission_mode(session.mode()).to_string(),
        tools: session.tool_names(),
        mcp_servers: session
            .mcp_status()
            .into_iter()
            .map(|line| (line.name, line.status))
            .collect(),
        slash_commands: catalog
            .all_commands()
            .into_iter()
            .map(|c| c.name.clone())
            .collect(),
        skills: catalog
            .all_skills()
            .into_iter()
            .map(|s| s.name.clone())
            .collect(),
        forge_version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_ids_must_be_uuids() {
        assert!(is_uuid("550e8400-e29b-41d4-a716-446655440000"));
        assert!(!is_uuid("550e8400e29b41d4a716446655440000"));
        assert!(!is_uuid("not-a-uuid"));
        assert!(!is_uuid("550e8400-e29b-41d4-a716-44665544000g"));
    }

    #[test]
    fn a_session_id_is_created_once_and_refused_when_taken() {
        let store = Store::open_in_memory().unwrap();
        let id = "550E8400-E29B-41D4-A716-446655440000";
        let created =
            create_session_with_id(&store, id, Some(forge_types::PermissionMode::Plan)).unwrap();
        assert_eq!(created, id.to_ascii_lowercase());
        assert!(store.session_exists(&created).unwrap());
        assert_eq!(store.session_mode(&created).unwrap(), "Plan");
        assert!(create_session_with_id(&store, id, None).is_err());
        assert!(create_session_with_id(&store, "nope", None).is_err());
    }
}
