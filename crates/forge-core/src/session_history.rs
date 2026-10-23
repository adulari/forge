//! Session history, checkpoints, replay, and workspace transition lifecycle.
//!
//! The methods retain their shared Session state and durable ordering guarantees.

use super::*;

impl Session {
    /// Rewind the conversation to a transcript boundary (`seq`): soft-delete the messages at/after
    /// it, restore any files those turns wrote (PR3 shadow snapshots), and truncate the live
    /// transcript. Returns the file-restore result plus the prompt that started the rewound-to turn
    /// (so the UI can put it back in the input box). Powers `/undo` and `/checkpoints`.
    /// `db_seq` is a DB **seq** (the stable identity checkpoints are keyed by), NOT a transcript
    /// index. The rewind menu's narrower actions are [`Self::rewind_scoped`].
    pub fn rewind_to(&mut self, db_seq: i64) -> Result<RewindOutcome, CoreError> {
        self.rewind_scoped(db_seq, crate::RewindScope::All)
    }

    /// Replace the in-memory transcript with the session's active rows and move `seq` past every
    /// row ever written (rewound rows stay in the table, so their seqs must not be reused).
    pub(crate) fn reload_transcript_from_store(&mut self) -> Result<(), CoreError> {
        let stored = self.store.load_messages(&self.id)?;
        self.seq = self.store.next_seq_for_session(&self.id)?;
        self.transcript = stored
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
        Ok(())
    }

    /// Re-derive every piece of session state that describes the transcript, after the transcript
    /// was rewritten from outside the model loop. Truncating the message list alone left the rest
    /// of the session believing the removed turns still existed: the bridge kept resuming its own
    /// server-side copy of them, the "already injected" latches kept AGENTS.md / white-hot guidance
    /// out even though the messages carrying them were gone, queued hints and images from the
    /// removed turn were delivered into the next one, and `/undo` chained from a stale turn seq.
    pub(crate) fn after_history_rewrite(&mut self) {
        self.history_epoch += 1;
        self.cache.note_history_rewrite();
        self.pending_hints.clear();
        self.shell_diagnoses.forget();
        self.lsp_baselines.clear();
        self.pending_images.clear();
        self.steer.clear();
        self.current_turn_seq = self
            .store
            .last_real_user_seq(&self.id, None)
            .ok()
            .flatten()
            .unwrap_or(-1);
        self.whitehot_guidance_injected = self
            .transcript
            .iter()
            .any(|m| m.role == Role::System && m.content == workflow::WHITEHOT_GUIDANCE);
        // Dropping the fingerprint makes the per-turn refresh read the file again on the next turn.
        self.project_prompt_injected = !self.transcript.is_empty();
        self.agents_md_fingerprint = None;
        // The rewound results may have carried directory instructions; the next touch re-sends them.
        self.instruction_scope = Default::default();
    }

    /// Undo the last user turn: rewind to (and including) the most recent user message, dropping
    /// that prompt and everything after it. `Ok(None)` if there's nothing to undo.
    pub fn undo(&mut self) -> Result<Option<RewindOutcome>, CoreError> {
        // Use current_turn_seq — the DB seq of the real user message that started this turn —
        // rather than the last user-role transcript message. The autofix stage injects synthetic
        // Role::User messages AFTER the real prompt (to feed lint/test failures back to the
        // model); landing on one would start the snapshot search too high and miss the snapshot
        // stored at current_turn_seq (causing restored: [] on undo). Sentinel -1 means no turn
        // has run yet.
        if self.current_turn_seq < 0 {
            return Ok(None);
        }
        let turn_seq = self.current_turn_seq;
        if self.store.user_prompt_at(&self.id, turn_seq)?.is_none() {
            return Ok(None);
        }
        // Locate the previous turn's user message before rewinding so chained undos work.
        let prev_turn_seq = self
            .store
            .last_real_user_seq(&self.id, Some(turn_seq))?
            .unwrap_or(-1);
        let outcome = self.rewind_to(turn_seq)?;
        self.current_turn_seq = prev_turn_seq;
        Ok(Some(outcome))
    }

    /// Build the current turn's snapshot context (session id, seq, absolute root, live temper) so the
    /// CLI bridge's `forge mcp-serve` child snapshots its writes into this turn's dir and matches the
    /// live permission mode.
    ///
    /// This is handed EXPLICITLY to the provider via [`CompletionOptions::checkpoint`], which applies
    /// it to the spawned child's own `Command` env at the spawn site — the parent no longer mutates
    /// its process-global env. That removes two hazards of the old `std::env::set_var` handoff:
    ///   - a future concurrent-session host sharing this process clobbering another session's context
    ///     between the write and the child spawn, and
    ///   - `set_var` racing a concurrent `getenv` on another thread (undefined behavior).
    ///
    /// The child still reads the same `FORGE_CHECKPOINT_*` / `FORGE_PERMISSION_MODE` var names from
    /// ITS OWN environment — unchanged from the child's perspective. The live temper is read fresh
    /// here so a Plan→Auto-edit switch (plan approval) or SHIFT+TAB reaches `mcp-serve` rather than it
    /// falling back to the stale on-disk config mode.
    pub(crate) fn checkpoint_context(&self) -> forge_provider::CheckpointContext {
        let root = std::path::absolute(&self.checkpoint_root)
            .unwrap_or_else(|_| self.checkpoint_root.clone());
        forge_provider::CheckpointContext {
            session: self.id.clone(),
            seq: self.current_turn_seq,
            root: root.to_string_lossy().into_owned(),
            workspace: self.workspace.root().to_string_lossy().into_owned(),
            mode: self.temper().key().to_string(),
            epoch: self.history_epoch,
        }
    }

    /// Save a conversation checkpoint at the current boundary. `label` None = an auto checkpoint.
    pub fn checkpoint(&mut self, label: Option<&str>) -> Result<(), CoreError> {
        // The DB's next seq, not `self.seq`: side-call anchor rows push the DB ahead of the
        // in-memory counter, and rewind deactivates by DB seq.
        let seq = self.store.next_seq_for_session(&self.id)?;
        self.store.add_checkpoint(&self.id, label, seq)?;
        Ok(())
    }

    /// This session's saved checkpoints, newest first.
    pub fn checkpoints(&self) -> Result<Vec<forge_store::CheckpointRow>, CoreError> {
        Ok(self.store.list_checkpoints(&self.id)?)
    }

    /// Visible conversation history (user + non-empty assistant messages), oldest first, for
    /// redrawing the transcript into the TUI scrollback after a `/resume` swap.
    pub fn history(&self) -> Vec<(Role, String)> {
        self.transcript
            .iter()
            .filter(|m| {
                m.visibility.is_user_visible()
                    && matches!(m.role, Role::User | Role::Assistant)
                    && !m.content.trim().is_empty()
            })
            .map(|m| (m.role, m.content.clone()))
            .collect()
    }

    /// The full rehydrated transcript as renderable [`ReplayItem`](forge_types::ReplayItem)s for the
    /// TUI to redraw on resume — user prompts, assistant text, AND the tool calls/results between
    /// them, so a resumed agentic session reappears faithfully instead of as a sparse user-only
    /// echo (the old [`history`](Self::history) dropped every tool-only assistant turn). Tool
    /// results are matched back to their call's name via the `tool_call_id`.
    pub fn replay_items(&self) -> Vec<forge_types::ReplayItem> {
        messages_to_replay_items(&self.transcript)
    }

    /// Like [`replay_items`](Self::replay_items) but over the FULL original history (including
    /// messages that compaction folded away), read straight from the store rather than the
    /// model-facing in-memory transcript. This is what lets the USER scroll back through the entire
    /// untouched conversation after a resume, even though the model only ever saw the compacted
    /// view. Falls back to the in-memory transcript if the store read fails.
    pub fn replay_items_full(&self) -> Vec<forge_types::ReplayItem> {
        match self.store.load_all_messages(&self.id) {
            Ok(stored) => {
                let msgs: Vec<Message> = stored
                    .into_iter()
                    .map(|m| Message {
                        reasoning: None,
                        provider_items: None,
                        role: m.role,
                        content: m.content,
                        tool_calls: m.tool_calls,
                        tool_call_id: m.tool_call_id,
                        images: Vec::new(),
                        visibility: m.visibility,
                        kind: m.kind,
                    })
                    .collect();
                messages_to_replay_items(&msgs)
            }
            Err(_) => self.replay_items(),
        }
    }

    /// The newest `limit` messages as replay items, plus the session's total message count — what
    /// a resume renders into scrollback. The log ring is bounded, so rendering the whole history
    /// of a 20k-message session only to trim it again was seconds of markdown work per start.
    pub fn replay_items_tail(&self, limit: usize) -> (Vec<forge_types::ReplayItem>, usize) {
        match self.store.load_message_tail(&self.id, limit) {
            Ok((stored, total)) => {
                let msgs: Vec<Message> = stored
                    .into_iter()
                    .map(|m| Message {
                        reasoning: None,
                        provider_items: None,
                        role: m.role,
                        content: m.content,
                        tool_calls: m.tool_calls,
                        tool_call_id: m.tool_call_id,
                        images: Vec::new(),
                        visibility: m.visibility,
                        kind: m.kind,
                    })
                    .collect();
                (messages_to_replay_items(&msgs), total)
            }
            Err(_) => {
                let items = self.replay_items();
                let n = items.len();
                (items, n)
            }
        }
    }

    /// Whether this session was compacted at least once (its model context is a summary, not the
    /// full history) — the signal for offering "continue compacted vs reload full" on resume.
    pub fn was_compacted(&self) -> bool {
        self.store.session_has_compaction(&self.id).unwrap_or(false)
    }

    /// Replace the model-facing transcript with the FULL, uncompacted history — the user chose, on
    /// resume, to continue WITHOUT compaction so the model re-reads the entire original
    /// conversation. (It may exceed the window; the next turn's auto-compaction handles that, now
    /// that token counting is precise.) The user-visible scrollback already shows everything.
    pub fn reload_full_context(&mut self) -> Result<(), CoreError> {
        // Compacted-away rows come back, `/undo`-rewound ones do not: they are no longer part of
        // the conversation (`uncompact_session_store` keeps them removed for the same reason).
        let stored = self.store.load_messages_uncompacted(&self.id)?;
        // MAX(seq)+1, not the loaded count — the loaded rows include compacted-away ones, so the
        // count would reuse seqs / inflate the rewind offset (same class of bug as
        // Session::resume, which is correctly scoped).
        self.seq = self.store.next_seq_for_session(&self.id)?;
        self.transcript = stored
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
        self.history_epoch += 1;
        self.cache.note_history_rewrite();
        Ok(())
    }

    pub(crate) fn transition_workspace(
        &mut self,
        workspace: WorkspaceContext,
    ) -> Result<(), CoreError> {
        if self.tools.rebind_workspace(workspace.root()).is_err() {
            self.tools.bind_workspace(workspace.root());
        }
        self.workspace = workspace;
        // Files the old root's tracker saw are not this root's; its cached repo top is wrong here.
        self.git_hygiene = Default::default();
        self.worktree_home = None;
        *self
            .workspace_binding
            .write()
            .map_err(|_| CoreError::Internal("session workspace binding poisoned".to_string()))? =
            self.workspace.root().to_path_buf();
        if !self.checkpoint_root_custom {
            self.checkpoint_root = crate::snapshot::default_root(self.workspace.root());
        }
        self.cached_git_branch = current_git_branch(self.workspace.root());
        self.set_instructions(crate::instructions::load(self.workspace.root()));
        self.agents_md_fingerprint = None;
        self.instruction_scope = Default::default();
        let (project, project_diagnostic) =
            crate::project_context::compute_with_diagnostic(self.workspace.root());
        self.project = project;
        if let Some(diagnostic) = project_diagnostic {
            self.presenter.emit(PresenterEvent::Warning(diagnostic));
        }
        // Lattice instances and their tool capture a root at construction. Recreate the index
        // for B and drop A's watcher; watcher composition is rebuilt by the CLI owner.
        let had_lattice = self.lattice.is_some();
        self.lattice_watcher = None;
        self.lattice_watcher_handle = None;
        self.tools.remove("lattice");
        self.lattice = had_lattice.then(|| {
            let lattice = Arc::new(Lattice::new(Arc::clone(&self.store), self.workspace.root()));
            self.tools
                .register(Box::new(forge_tools::LatticeTool::new(Arc::clone(
                    &lattice,
                ))));
            lattice
        });
        if self.lattice_watch_enabled {
            self.install_lattice_watcher();
        }
        Ok(())
    }

    /// Reconfigure this session in place as a **fresh** one (new id, empty transcript), keeping
    /// the same backends + live presenter so events keep flowing to the running TUI. Powers
    /// `/new` — no process restart, no Session move (it lives behind the loop's `Mutex`).
    pub fn reset_fresh(&mut self, cwd: &str) -> Result<(), CoreError> {
        let workspace = WorkspaceContext::new(cwd)?;
        let id = self
            .store
            .create_session(&workspace.display(), format!("{:?}", self.mode).as_str())?;
        self.transition_workspace(workspace)?;
        self.id = id.clone();
        self.frozen_cache_key = None;
        self.transcript.clear();
        self.seq = 0;
        self.tasks.clear();
        self.forget_context_shape();
        self.project_prompt_injected = false;
        self.pinned = Default::default();
        self.pinned.recall_due = true;
        self.recent_files = Default::default();
        self.set_instructions(crate::instructions::load(self.workspace.root()));
        self.presenter.emit(PresenterEvent::SessionStarted { id });
        self.emit_session_totals(true);
        Ok(())
    }

    /// Reconfigure this session in place, **resumed** from `session_id`: rehydrate the stored
    /// transcript, keep the same backends + live presenter. Powers `/resume`.
    pub fn reset_resumed(&mut self, session_id: &str) -> Result<(), CoreError> {
        if !self.store.session_exists(session_id)? {
            return Err(CoreError::SessionNotFound(session_id.to_string()));
        }
        let cwd = self
            .store
            .session_cwd(session_id)?
            .ok_or_else(|| CoreError::SessionNotFound(session_id.to_string()))?;
        let workspace = WorkspaceContext::new(cwd)?;
        let stored = self.store.load_messages(session_id)?;
        // MAX(seq)+1, not the loaded count — see Session::resume (compaction makes them differ, and
        // the mismatch lets `/undo` deactivate pre-compaction survivors).
        self.seq = self.store.next_seq_for_session(session_id)?;
        self.transcript = stored
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
        self.transition_workspace(workspace)?;
        self.id = session_id.to_string();
        // Same restore as `Session::resume`: the picker/`/resume` path used to come back at the
        // launch mode and routing instead of the session's own.
        if let Some(mode) = self
            .store
            .session_mode(session_id)
            .ok()
            .and_then(|stored| crate::session_lifecycle::parse_stored_mode(&stored))
        {
            self.set_mode(mode);
        }
        let stored_effort = self
            .store
            .session_pinned_effort(session_id)
            .ok()
            .flatten()
            .and_then(|stored| EffortLevel::parse(&stored));
        self.set_effort(stored_effort);
        let stored_pin = self.store.session_pinned_model(session_id).ok().flatten();
        self.pin_model(stored_pin);
        self.subagent_pin_free = self
            .store
            .session_subagent_pin_free(session_id)
            .ok()
            .flatten();
        self.forget_context_shape();
        self.frozen_cache_key = None;
        self.tasks = match self.store.tasks(session_id) {
            Ok(tasks) => tasks,
            Err(error) => {
                tracing::warn!(session_id, %error, "session task history could not be restored");
                Vec::new()
            }
        };
        self.project_prompt_injected = true;
        self.pinned = Default::default();
        self.pinned.recall_due = true;
        self.recent_files = Default::default();
        self.refresh_pinned_tasks();
        self.agents_md_fingerprint = None;
        self.instruction_scope = Default::default();
        self.presenter.emit(PresenterEvent::SessionStarted {
            id: session_id.to_string(),
        });
        self.emit_session_totals(true);
        // Re-show the restored task list so the resumed session's progress is visible.
        if !self.tasks.is_empty() {
            self.presenter
                .emit(PresenterEvent::Tasks(self.tasks.clone()));
        }
        Ok(())
    }
}
