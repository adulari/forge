//! Session construction and resume lifecycle.
//!
//! This module owns the only transitions that establish a session's durable
//! identity, transcript sequence, cached workspace metadata, and presenter
//! start event. Turn orchestration remains in the session program.

use std::sync::Arc;

use forge_config::Config;
use forge_mesh::{pricing::Pricing, Router};
use forge_provider::Provider;
use forge_store::Store;
use forge_tools::ToolRegistry;
use forge_types::{EffortLevel, Message, PermissionMode, Presenter, PresenterEvent};

use crate::{
    context_pack, current_git_branch, read_project_agents_md, turn_contract, CoreError,
    EnvFightTracker, Session, ToolFailureTracker, WorkspaceContext,
};

impl Session {
    pub fn start(
        store: Arc<Store>,
        provider: Arc<dyn Provider>,
        router: Arc<dyn Router>,
        tools: ToolRegistry,
        presenter: Box<dyn Presenter>,
        config: Config,
        cwd: &str,
    ) -> Result<Self, CoreError> {
        let workspace = WorkspaceContext::new(cwd)?;
        let mode = config.permission_mode;
        let id = store.create_session(&workspace.display(), format!("{mode:?}").as_str())?;
        Ok(Self::build(
            id,
            store,
            provider,
            router,
            tools,
            presenter,
            config,
            workspace,
            Vec::new(),
            0,
        ))
    }

    /// Resume an existing session: rehydrate its transcript and continue the same row.
    #[allow(clippy::too_many_arguments)]
    pub fn resume(
        store: Arc<Store>,
        provider: Arc<dyn Provider>,
        router: Arc<dyn Router>,
        tools: ToolRegistry,
        presenter: Box<dyn Presenter>,
        config: Config,
        session_id: &str,
    ) -> Result<Self, CoreError> {
        if !store.session_exists(session_id)? {
            return Err(CoreError::SessionNotFound(session_id.to_string()));
        }
        let stored = store.load_messages(session_id)?;
        // The next seq is MAX(seq)+1 from the DB, NOT the loaded count — after compaction
        // `load_messages` returns only the active tail (+ summary), so its length is far below the
        // real max. Using the count would reuse low seqs and make `/undo` wipe pre-compaction history.
        let seq = store.next_seq_for_session(session_id)?;
        let cwd = store
            .session_cwd(session_id)?
            .ok_or_else(|| CoreError::SessionNotFound(session_id.to_string()))?;
        let workspace = WorkspaceContext::new(cwd)?;
        let transcript = stored
            .into_iter()
            .map(|m| Message {
                role: m.role,
                content: m.content,
                tool_calls: m.tool_calls,
                tool_call_id: m.tool_call_id,
                images: Vec::new(),
                visibility: m.visibility,
            })
            .collect();
        // Restore the permission mode that was active when the session was last saved.
        let mut config = config;
        if let Ok(stored_mode) = store.session_mode(session_id) {
            let parsed = match stored_mode.as_str() {
                "Default" => Some(PermissionMode::Default),
                "AcceptEdits" => Some(PermissionMode::AcceptEdits),
                "Bypass" => Some(PermissionMode::Bypass),
                "Plan" => Some(PermissionMode::Plan),
                _ => PermissionMode::from_label(&stored_mode),
            };
            if let Some(m) = parsed {
                config.permission_mode = m;
            }
        }
        // Restore the reasoning-effort pin the same way, and for the same reason: without it a
        // session driven at a raised effort silently comes back at the provider default, which
        // reads as the agent quietly downgrading itself mid-goal.
        let pinned_effort = store
            .session_pinned_effort(session_id)
            .ok()
            .flatten()
            .and_then(|stored| EffortLevel::parse(&stored));
        let mut session = Self::build(
            session_id.to_string(),
            store,
            provider,
            router,
            tools,
            presenter,
            config,
            workspace,
            transcript,
            seq,
        );
        if let Some(level) = pinned_effort {
            session.set_effort(Some(level));
        }
        Ok(session)
    }

    #[allow(clippy::too_many_arguments)]
    fn build(
        id: String,
        store: Arc<Store>,
        provider: Arc<dyn Provider>,
        router: Arc<dyn Router>,
        tools: ToolRegistry,
        presenter: Box<dyn Presenter>,
        config: Config,
        workspace: WorkspaceContext,
        transcript: Vec<Message>,
        seq: i64,
    ) -> Self {
        let mode = config.permission_mode;
        // Layer fetched per-model prices (OpenRouter etc., persisted at discovery) under the config
        // overrides, so gateway/credit spend is priced instead of silently $0 (the budget cap and
        // the /usage breakdown both read these computed costs).
        let fetched_prices = store.all_model_pricing().unwrap_or_default();
        let pricing = Pricing::from_config_with_fetched(&config, fetched_prices);
        let rules = config.permission_rules();
        // Rehydrate the task list (empty for a fresh session; restored on resume).
        let tasks = store.tasks(&id).unwrap_or_default();
        // Resumed sessions already have AGENTS.md in the stored transcript; don't re-inject.
        let project_prompt_injected = !transcript.is_empty();
        let checkpoint_root = workspace.root().join(".forge/checkpoints");
        let cached_git_branch = current_git_branch(workspace.root());
        let cached_agents_md = if project_prompt_injected {
            None
        } else {
            read_project_agents_md(workspace.root())
        };
        let project = crate::project_context::compute(workspace.root());
        let mut s = Self {
            id,
            store,
            provider,
            router,
            tools,
            presenter,
            config,
            pricing,
            mode,
            pre_plan_mode: None,
            rules,
            transcript,
            seq,
            checkpoint_root,
            checkpoint_root_custom: false,
            current_turn_seq: -1,
            catalog: None,
            tasks,
            pending_plan: None,
            task_scope: None,
            mcp: None,
            lattice: None,
            lattice_watcher: None,
            lattice_watch_enabled: false,
            lsp: None,
            skills: None,
            pinned_model: None,
            pinned_effort: None,
            overflow_window_cap: None,
            whitehot_guidance_injected: false,
            pinned_tier: None,
            route_affinity: None,
            workspace_binding: Arc::new(std::sync::RwLock::new(workspace.root().to_path_buf())),
            workspace,
            pending_hints: vec![],
            always_compact_on_switch: false,
            project_prompt_injected,
            pending_images: Vec::new(),
            edits_this_turn: 0,
            expect_code_change: false,
            tools_unavailable_run: false,
            turn_deadline: None,
            deadline_reconciled: false,
            env_fight: EnvFightTracker::default(),
            failure_tracker: ToolFailureTracker::new(),
            cached_git_branch,
            // Read AGENTS.md eagerly (sync, off the async path) only when it will actually be
            // injected — i.e. a fresh session. A resumed session already has it in the transcript.
            cached_agents_md,
            project,
            last_context_pack: context_pack::ContextPack::default(),
            last_turn_contract: turn_contract::TurnContract::default(),
        };
        let id = s.id.clone();
        s.presenter.emit(PresenterEvent::SessionStarted { id });
        s
    }
}
