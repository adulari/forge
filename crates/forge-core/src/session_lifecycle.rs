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
    context_pack, current_git_branch, normalize_workspace_target, turn_contract, CoreError,
    EnvFightTracker, Session, WorkspaceContext,
};

/// Shown when a resumed session's previous process died without clearing its terminal claim. A
/// surface that claims the session before resuming it consumes that signal, and shows this itself.
pub const UNCLEAN_EXIT_NOTICE: &str = "the previous run of this session ended without a clean \
     exit (killed or crashed); its last turn may be incomplete";

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
        let mut session = Self::build(
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
        );
        session.record_host_on_start();
        Ok(session)
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
        // Retained async subagents (RFC retained-async-subagents): a row still `running` belonged
        // to a task in whatever process last admitted it. This resume is a FRESH process (the
        // common case is a daemon restart or `forge run --resume`), so that task is gone and will
        // never call `finish_detached_child` — reconcile it to `failed` now, once, before this
        // session's `detached_registry` (empty — no live tasks) is trusted as authoritative. Must
        // NOT be called again on every turn: a legitimately still-running detached child in THIS
        // process would otherwise be swept out from under itself.
        let _ = store.reconcile_running_detached_children(session_id);
        let stored = store.load_messages(session_id)?;
        // The next seq is MAX(seq)+1 from the DB, NOT the loaded count — after compaction
        // `load_messages` returns only the active tail (+ summary), so its length is far below the
        // real max. Using the count would reuse low seqs and make `/undo` wipe pre-compaction history.
        let seq = store.next_seq_for_session(session_id)?;
        let cwd = store
            .session_cwd(session_id)?
            .ok_or_else(|| CoreError::SessionNotFound(session_id.to_string()))?;
        // A session moved to another host may record a cwd that does not exist here.
        let cwd = crate::host_move::resolve_resume_cwd(&store, session_id, &cwd);
        let workspace = WorkspaceContext::new(cwd)?;
        let transcript = stored
            .into_iter()
            .map(|m| Message {
                reasoning: None,
                provider_items: m.provider_items,
                role: m.role,
                content: m.content,
                tool_calls: m.tool_calls,
                tool_call_id: m.tool_call_id,
                images: Vec::new(),
                visibility: m.visibility,
                kind: m.kind,
            })
            .collect();
        // Restore the permission mode that was active when the session was last saved.
        let mut config = config;
        if let Ok(stored_mode) = store.session_mode(session_id) {
            let parsed = parse_stored_mode(&stored_mode);
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
        let pinned_model = store.session_pinned_model(session_id).ok().flatten();
        let subagent_pin_free = store.session_subagent_pin_free(session_id).ok().flatten();
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
        if pinned_model.is_some() {
            session.pin_model(pinned_model);
        }
        // Same reasoning for `/subagents free`: a resumed goal run that had released its children
        // from the pin would otherwise quietly drag every child back onto the parent's model.
        session.subagent_pin_free = subagent_pin_free;
        // Last, so the notice lands after the restored transcript (append-only, cache-friendly).
        session.reconcile_host_on_resume();
        // Seed the context gauge and totals from the store so a restored session does not report
        // an empty gauge until its next model call.
        session.refresh_gauge_after_history_change_from_hint();
        if session.store.take_unclean_exit(session_id).unwrap_or(false) {
            session
                .presenter
                .emit(PresenterEvent::Warning(UNCLEAN_EXIT_NOTICE.to_string()));
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
        mut presenter: Box<dyn Presenter>,
        config: Config,
        workspace: WorkspaceContext,
        transcript: Vec<Message>,
        seq: i64,
    ) -> Self {
        let mode = config.permission_mode;
        let provider = crate::zero_retention::guard(provider, &config);
        // Opt-in allowlist (`tools.extra_roots`) of extra roots the structured file tools may
        // read and write, in addition to the workspace. Relative entries are ignored. Empty by
        // default, so this is a no-op for existing configs.
        let extra_tool_roots: Vec<std::path::PathBuf> = config
            .tools
            .effective_extra_roots()
            .iter()
            .map(std::path::Path::new)
            .map(normalize_workspace_target)
            .collect();
        let _ = tools.bind_extra_roots(extra_tool_roots.clone());
        // Layer fetched per-model prices (OpenRouter etc., persisted at discovery) under the config
        // overrides, so gateway/credit spend is priced instead of silently $0 (the budget cap and
        // the /usage breakdown both read these computed costs).
        let fetched_prices = store.all_model_pricing().unwrap_or_default();
        let pricing = Pricing::from_config_with_fetched(&config, fetched_prices);
        let rules = config.permission_rules();
        // Rehydrate the task list (empty for a fresh session; restored on resume).
        let tasks = match store.tasks(&id) {
            Ok(tasks) => tasks,
            Err(error) => {
                tracing::warn!(session_id = %id, %error, "session task history could not be restored");
                Vec::new()
            }
        };
        // Resumed sessions already have AGENTS.md in the stored transcript; don't re-inject.
        let project_prompt_injected = !transcript.is_empty();
        let checkpoint_root = crate::snapshot::default_root(workspace.root());
        let cached_git_branch = current_git_branch(workspace.root());
        let instructions = crate::instructions::load(workspace.root());
        let agents_md_fingerprint = Some(crate::instructions::fingerprint(&instructions.watched));
        let (project, project_diagnostic) =
            crate::project_context::compute_with_diagnostic(workspace.root());
        if let Some(diagnostic) = project_diagnostic {
            presenter.emit(PresenterEvent::Warning(diagnostic));
        }
        let mut s = Self {
            id,
            store,
            provider,
            frozen_cache_key: None,
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
            catalog_feed: None,
            catalog_feed_seen: None,
            tasks,
            pending_plan: None,
            task_scope: None,
            mcp: None,
            lattice: None,
            lattice_watcher: None,
            lattice_watcher_handle: None,
            lattice_update: None,
            lattice_watch_enabled: false,
            lsp: None,
            lsp_baselines: Default::default(),
            skills: None,
            fleet: None,
            dispatch: None,
            pinned_model: None,
            pinned_effort: None,
            exact_effort: None,
            turn_grant: Default::default(),
            adaptive_effort: Default::default(),
            window_lookup: None,
            subagent_pin_free: None,
            overflow_window_cap: None,
            require_tool_call: false,
            repetition_pressure: 0,
            ledger: Default::default(),
            kept_outputs: std::collections::HashMap::new(),
            whitehot_guidance_injected: false,
            pinned_tier: None,
            route_affinity: None,
            workspace_binding: Arc::new(std::sync::RwLock::new(workspace.root().to_path_buf())),
            worktree_home: None,
            workspace,
            extra_tool_roots,
            btw_asker: None,
            pending_hints: vec![],
            shell_diagnoses: Default::default(),
            steer: crate::steer::SteerInbox::default(),
            resume_absorbed: false,
            job_wake: crate::job_wake::JobWake::default(),
            git_hygiene: crate::git_hygiene::Tracker::default(),
            stale_tasks: crate::task_staleness::Tracker::default(),
            history_epoch: 0,
            transcript_origin: 0,
            compactions: 0,
            cache: Default::default(),
            context_floor: Default::default(),
            context_trim: Default::default(),
            tools_advertised_once: false,
            headroom_args_warned: false,
            auto_classifier_cache: Default::default(),
            always_compact_on_switch: false,
            bridge_context_tokens: 0,
            project_prompt_injected,
            agents_md_fingerprint,
            instruction_scope: Default::default(),
            pending_images: Vec::new(),
            vision: Default::default(),
            edits_this_turn: 0,
            mutations_this_turn: 0,
            turn_start_baseline: Default::default(),
            turn_input_tokens: 0,
            turn_billable_input_tokens: 0,
            bridge_trace: Default::default(),
            bridge_stall: None,
            turn_output_tokens: 0,
            turn_hard_guard_abort: false,
            run_budget: None,
            run_budget_next: None,
            fallback_models: Vec::new(),
            turn_forced_stop: None,
            harness_profile: forge_config::HarnessProfile::default(),
            empty_reply_nudged: false,
            trim_watch: Default::default(),
            gauge_model: None,
            spend: Default::default(),
            turn_unfinished_tasks: Vec::new(),
            turn_budget_refused: false,
            expect_code_change: false,
            cli_bridge_notice_shown: false,
            tools_unavailable_run: false,
            turn_deadline: None,
            deadline_reconciled: false,
            env_fight: EnvFightTracker::default(),
            cached_git_branch,
            attribution: Default::default(),
            // Set below, through `set_instructions`.
            instructions: Default::default(),
            compact_guard: Default::default(),
            pinned: Default::default(),
            recent_files: Default::default(),
            project,
            last_context_pack: context_pack::ContextPack::default(),
            last_turn_contract: turn_contract::TurnContract::default(),
            detached_registry: crate::subagent::DetachedRegistry::new(),
            turns_since_refine: 0,
            one_shot: false,
            default_commit_policy: forge_config::CommitPolicy::Unit,
            unverified_models: Vec::new(),
            unavailable_models: Vec::new(),
            idle_nudge_carry: (0, 0),
            host_guard: Default::default(),
            compact_directive: None,
            compact_timeout: crate::compaction_run::COMPACT_CALL_TIMEOUT,
        };
        s.pinned.recall_due = true;
        // Read eagerly (sync, off the async path): the pinned block carries the instruction files
        // on every request, a resumed session included.
        s.set_instructions(instructions);
        s.refresh_pinned_tasks();
        let id = s.id.clone();
        s.presenter.emit(PresenterEvent::SessionStarted { id });
        s
    }
}

/// A session's stored permission-mode string back into the enum (legacy debug names and labels).
pub(crate) fn parse_stored_mode(stored: &str) -> Option<PermissionMode> {
    match stored {
        "Default" => Some(PermissionMode::Default),
        "AcceptEdits" => Some(PermissionMode::AcceptEdits),
        "Bypass" => Some(PermissionMode::Bypass),
        "Plan" => Some(PermissionMode::Plan),
        "Auto" => Some(PermissionMode::Auto),
        _ => PermissionMode::from_label(stored),
    }
}
