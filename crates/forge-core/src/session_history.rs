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
    /// index — both `/undo` and the `/checkpoints` picker pass a seq. After a COMPACTED resume the
    /// in-memory transcript is just the active tail while the DB seqs start high, so the two diverge;
    /// `offset` (0 when not compacted) maps the seq back to the transcript index for truncation.
    pub fn rewind_to(&mut self, db_seq: i64) -> Result<RewindOutcome, CoreError> {
        let db_seq = db_seq.max(0);
        // DB seq → transcript INDEX. Deactivation/snapshot work in DB seq; transcript ops in index.
        let offset = self.seq - self.transcript.len() as i64;
        let idx = (db_seq - offset).max(0) as usize;
        // The message AT the boundary is the user prompt of the rewound-to turn; capture it before
        // truncation so the UI can re-offer it for editing/resubmitting.
        let rewound_prompt = self
            .transcript
            .get(idx)
            .filter(|m| m.role == Role::User)
            .map(|m| m.content.clone());
        let mut restore = snapshot::RestoreReport::default();
        // Turns are keyed by their user-message seq. Restore every snapshotted turn at/after the
        // boundary, newest first so an earlier turn's blob (pre-turn bytes) wins the final state.
        for seq in (db_seq..self.seq).rev() {
            match snapshot::restore_turn(&self.checkpoint_root, &self.id, seq) {
                Ok(r) => {
                    restore.restored.extend(r.restored);
                    restore.warnings.extend(r.warnings);
                    restore.failed.extend(r.failed);
                }
                // Surface the failure instead of silently discarding it — a caller that only
                // checks `restore.restored`/`is_empty()` must still learn this turn's files may
                // not have been reverted.
                Err(e) => restore.failed.push(format!("turn {seq}: {e}")),
            }
        }
        self.store.deactivate_messages_from(&self.id, db_seq)?;
        self.transcript.truncate(idx);
        self.seq = db_seq;
        Ok(RewindOutcome {
            restore,
            rewound_prompt,
        })
    }

    /// Undo the last user turn: rewind to (and including) the most recent user message, dropping
    /// that prompt and everything after it. `Ok(None)` if there's nothing to undo.
    pub fn undo(&mut self) -> Result<Option<RewindOutcome>, CoreError> {
        // Use current_turn_seq — the DB seq of the real user message that started this turn —
        // rather than rposition(Role::User). The autofix stage injects synthetic Role::User
        // messages AFTER the real prompt (to feed lint/test failures back to the model); rposition
        // would land on the synthetic message, making rewind_to start the snapshot search too high
        // and miss the snapshot stored at current_turn_seq (causing restored: [] on undo).
        //
        // transcript_idx = db_seq - offset  (offset = self.seq - len absorbs compaction gaps so
        // the mapping stays valid after resume). Sentinel -1 means no turn has run yet.
        if self.current_turn_seq < 0 {
            return Ok(None);
        }
        let offset = self.seq - self.transcript.len() as i64;
        let turn_idx = (self.current_turn_seq - offset).max(0) as usize;
        if self
            .transcript
            .get(turn_idx)
            .filter(|m| m.role == Role::User)
            .is_none()
        {
            return Ok(None);
        }
        // Locate the previous turn's user message before rewinding so chained undos work.
        let prev_turn_seq = self.transcript[..turn_idx]
            .iter()
            .rposition(|m| m.role == Role::User)
            .map(|p| p as i64 + offset)
            .unwrap_or(-1);
        let outcome = self.rewind_to(self.current_turn_seq)?;
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
        }
    }

    /// Save a conversation checkpoint at the current boundary. `label` None = an auto checkpoint.
    pub fn checkpoint(&mut self, label: Option<&str>) -> Result<(), CoreError> {
        self.store.add_checkpoint(&self.id, label, self.seq)?;
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
                        role: m.role,
                        content: m.content,
                        tool_calls: m.tool_calls,
                        tool_call_id: m.tool_call_id,
                        images: Vec::new(),
                        visibility: m.visibility,
                    })
                    .collect();
                messages_to_replay_items(&msgs)
            }
            Err(_) => self.replay_items(),
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
        let stored = self.store.load_all_messages(&self.id)?;
        // MAX(seq)+1, not the loaded count — `load_all_messages` includes soft-deleted rows from prior
        // rewinds, so its length exceeds the real max seq and the count would reuse seqs / inflate the
        // rewind offset (same class of bug as Session::resume, which is correctly scoped).
        self.seq = self.store.next_seq_for_session(&self.id)?;
        self.transcript = stored
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
        Ok(())
    }

    fn transition_workspace(&mut self, workspace: WorkspaceContext) -> Result<(), CoreError> {
        if self.tools.rebind_workspace(workspace.root()).is_err() {
            self.tools.bind_workspace(workspace.root());
        }
        self.workspace = workspace;
        *self
            .workspace_binding
            .write()
            .map_err(|_| CoreError::Internal("session workspace binding poisoned".to_string()))? =
            self.workspace.root().to_path_buf();
        if !self.checkpoint_root_custom {
            self.checkpoint_root = self.workspace.root().join(".forge/checkpoints");
        }
        self.cached_git_branch = current_git_branch(self.workspace.root());
        self.cached_agents_md = if self.project_prompt_injected {
            None
        } else {
            read_project_agents_md(self.workspace.root())
        };
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
        self.transcript.clear();
        self.seq = 0;
        self.tasks.clear();
        self.project_prompt_injected = false;
        self.cached_agents_md = read_project_agents_md(self.workspace.root());
        self.presenter.emit(PresenterEvent::SessionStarted { id });
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
                role: m.role,
                content: m.content,
                tool_calls: m.tool_calls,
                tool_call_id: m.tool_call_id,
                images: Vec::new(),
                visibility: m.visibility,
            })
            .collect();
        self.transition_workspace(workspace)?;
        self.id = session_id.to_string();
        self.tasks = match self.store.tasks(session_id) {
            Ok(tasks) => tasks,
            Err(error) => {
                tracing::warn!(session_id, %error, "session task history could not be restored");
                Vec::new()
            }
        };
        self.project_prompt_injected = true;
        self.presenter.emit(PresenterEvent::SessionStarted {
            id: session_id.to_string(),
        });
        // Re-show the restored task list so the resumed session's progress is visible.
        if !self.tasks.is_empty() {
            self.presenter
                .emit(PresenterEvent::Tasks(self.tasks.clone()));
        }
        Ok(())
    }
}
