//! What a remote client's input does to a daemon-hosted session.
//!
//! This is the headless mirror of the TUI's remote drain: the same routing for `//` escapes,
//! `/command` dispatch, and plain prompts, minus the cases that only mean something on a host
//! terminal. Two behaviours are load-bearing and live here because they are decided at input
//! time, not turn time: input that arrives while a turn is running is queued rather than dropped,
//! and a `Prompt`'s own message-correlated attachment list is authoritative for that turn (the
//! mobile upload race), overriding ambient state from an unrelated earlier `Attach`.

use super::*;
use crate::cli::commands::run::queue_edit;

impl DriverState {
    /// One remote input — the headless mirror of `run_chat_tui`'s remote drain, minus the
    /// host-terminal cases (`/remote` toggling, host clipboard).
    pub(super) async fn handle_input(&mut self, input: remote::RemoteInput) -> Result<()> {
        if self.draining.load(Ordering::Acquire)
            && matches!(
                input,
                remote::RemoteInput::Prompt { .. }
                    | remote::RemoteInput::Steer { .. }
                    | remote::RemoteInput::FollowUp { .. }
                    | remote::RemoteInput::Queue { .. }
            )
        {
            return Ok(());
        }
        match input {
            remote::RemoteInput::Prompt { text, attachments } => {
                self.handle_prompt(text, attachments, true).await?;
            }
            remote::RemoteInput::Queue { text, attachments } => {
                self.handle_prompt(text, attachments, false).await?;
            }
            remote::RemoteInput::QueueMove { index, text, to } => {
                if queue_edit::move_queued(&mut self.queued_prompts, &self.steer, index, &text, to)
                {
                    self.app.set_queued(&self.queued_prompts);
                } else {
                    push_remote_note(&mut self.notes, queue_edit::STALE_QUEUE_EDIT_NOTE);
                }
            }
            remote::RemoteInput::QueueSendNow { index, text } => {
                if self.busy {
                    if queue_edit::move_queued(
                        &mut self.queued_prompts,
                        &self.steer,
                        index,
                        &text,
                        0,
                    ) {
                        self.app.set_queued(&self.queued_prompts);
                        Box::pin(self.handle_input(remote::RemoteInput::Interrupt)).await?;
                    } else {
                        push_remote_note(&mut self.notes, queue_edit::STALE_QUEUE_EDIT_NOTE);
                    }
                }
            }
            remote::RemoteInput::Allow { yes, seq, note } => {
                if !remote::prompt_seq_current(self.prompt_seq, seq) {
                    push_remote_note(
                        &mut self.notes,
                        "⚠ stale answer ignored — the prompt changed; review the current one",
                    );
                } else if let Some((tool, reply)) = self.pending.take() {
                    let _ = reply.send(remote::allow_reply(yes, note.as_deref()));
                    self.app.prompt = None;
                    if yes {
                        self.app.note(&format!("✓ remote allowed {tool}"));
                    } else {
                        self.app.note(&format!("✗ remote denied {tool}"));
                    }
                }
            }
            remote::RemoteInput::Answer { text, seq } => {
                if !remote::prompt_seq_current(self.prompt_seq, seq) {
                    push_remote_note(
                        &mut self.notes,
                        "⚠ stale answer ignored — the prompt changed; review the current one",
                    );
                } else if self.app.awaiting_question() {
                    match self.app.resolve_remote_answer(&text) {
                        forge_tui::app::RemoteAnswer::Complete(answers) => {
                            if let Some(tx) = self.pending_question.take() {
                                let _ = tx.send(answers);
                            }
                        }
                        forge_tui::app::RemoteAnswer::Partial => {}
                        forge_tui::app::RemoteAnswer::Invalid => {
                            self.app.note("⚠ remote answer was invalid — re-asking");
                        }
                    }
                } else {
                    push_remote_note(
                        &mut self.notes,
                        "⚠ no question is waiting — answer a permission prompt with Allow/Deny",
                    );
                }
            }
            remote::RemoteInput::Interrupt => {
                if self.busy {
                    let note = if self.interrupt_turn() {
                        "⏹ remote interrupted — continuing with the next queued prompt"
                    } else {
                        crate::cli::commands::run::INTERRUPTED_NOTE
                    };
                    self.app.note(note);
                    self.drain_queue().await;
                }
            }
            remote::RemoteInput::Dequeue { index, text } => {
                let idx = index as usize;
                if idx < self.queued_prompts.len() && self.queued_prompts[idx] == text {
                    self.queued_prompts.remove(idx);
                    self.steer.remove_pending(&text);
                    self.app.set_queued(&self.queued_prompts);
                    self.app.note(&format!(
                        "✕ remote dequeued — {} pending",
                        self.queued_prompts.len()
                    ));
                } else {
                    push_remote_note(
                        &mut self.notes,
                        "⚠ stale dequeue ignored — the queue changed; review the current list",
                    );
                }
            }
            remote::RemoteInput::Key { key } => {
                // Same guards as the TUI drain: prompts resolve ONLY via seq-checked
                // Allow/Answer, and a bare idle Esc must never do anything drastic.
                if self.pending.is_some() || self.app.awaiting_question() {
                    push_remote_note(
                        &mut self.notes,
                        "⚠ a prompt is pending — answer it with its buttons",
                    );
                } else {
                    match remote::named_key(&key) {
                        Some(KeyKind::Esc) if !self.busy && !any_remote_modal_open(&self.app) => {
                            push_remote_note(&mut self.notes, "Esc ignored — nothing to close");
                        }
                        Some(k) => self.remote_keys.push_back(k),
                        None => push_remote_note(
                            &mut self.notes,
                            &format!("⚠ unknown key {key:?} ignored"),
                        ),
                    }
                }
            }
            remote::RemoteInput::OverlaySelect { id } => {
                let keys = apply_overlay_input(&mut self.app, RemoteOverlayOp::Select(id));
                self.remote_keys.extend(keys);
            }
            remote::RemoteInput::OverlayNav { delta } => {
                let keys = apply_overlay_input(&mut self.app, RemoteOverlayOp::Nav(delta));
                self.remote_keys.extend(keys);
            }
            remote::RemoteInput::OverlayFilter { text } => {
                let keys = apply_overlay_input(&mut self.app, RemoteOverlayOp::Filter(text));
                self.remote_keys.extend(keys);
            }
            remote::RemoteInput::OverlayCancel => {
                let keys = apply_overlay_input(&mut self.app, RemoteOverlayOp::Cancel);
                self.remote_keys.extend(keys);
            }
            remote::RemoteInput::Attach { path, image } => {
                let cwd = self.cwd.clone();
                handle_remote_attach(
                    &self.session,
                    &mut self.app,
                    &mut self.pending_mentions,
                    &cwd,
                    path,
                    image,
                )
                .await;
            }
            remote::RemoteInput::FollowUp { text } => {
                if self.busy && is_command_line(&text) {
                    self.busy_command(text).await?;
                } else if self.busy {
                    self.prompt_history.record(&text);
                    forge_core::fleet::insert_into_queue(
                        &mut self.queued_prompts,
                        forge_core::fleet::MessageMode::FollowUp,
                        text,
                    );
                    self.app.set_queued(&self.queued_prompts);
                    self.app
                        .note("⏳ follow-up queued — runs when this turn finishes");
                } else {
                    self.submit_line(text, None).await?;
                }
            }
            remote::RemoteInput::Steer { text } => {
                if self.busy && is_command_line(&text) {
                    self.busy_command(text).await?;
                } else if self.busy {
                    self.steer.push_front(text.clone());
                    forge_core::fleet::insert_into_queue(
                        &mut self.queued_prompts,
                        forge_core::fleet::MessageMode::Steer,
                        text,
                    );
                    self.app.set_queued(&self.queued_prompts);
                    self.app.note("⚡ steered — sent at this turn's next step");
                } else {
                    self.submit_line(text, None).await?;
                }
            }
        }
        Ok(())
    }

    /// A prompt typed by a remote client. Mid-turn a plain prompt is queued, and when `steer` is
    /// set also offered to the running turn at its next step; `steer = false` only queues it.
    async fn handle_prompt(
        &mut self,
        text: String,
        attachments: Vec<remote::PromptAttachment>,
        steer: bool,
    ) -> Result<()> {
        // A fresh prompt starts a fresh interaction — drop stale notices + copy payload.
        self.notes.clear();
        self.copy_text = None;

        // A message-correlated attachment list (mobile upload race fix, mirrors
        // `run_chat_tui`'s remote drain) is authoritative for THIS turn when non-empty:
        // discard stale ambient state from an unrelated `Attach` up front (image
        // attachment happens inside `resolve_prompt_attachments` regardless of dispatch
        // branch, matching how it already applied ambiently); non-image mentions are
        // threaded into `submit_line` and only actually prepended in its plain-prompt
        // branch, exactly where the old ambient `pending_mentions` were.
        let has_explicit_attachments = !attachments.is_empty();
        if has_explicit_attachments {
            self.pending_mentions.clear();
        }
        let explicit_mentions = resolve_prompt_attachments(
            &self.session,
            &mut self.app,
            &mut self.notes,
            &self.cwd,
            attachments,
        )
        .await;

        if self.busy {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                // nothing to queue
            } else if is_command_line(trimmed) {
                self.busy_command(text).await?;
            } else {
                self.prompt_history.record(&text);
                queue_prompt(
                    &mut self.queued_prompts,
                    &self.steer,
                    &mut self.app,
                    text,
                    Vec::new(),
                    steer,
                );
            }
            return Ok(());
        }
        self.submit_line(text, has_explicit_attachments.then_some(explicit_mentions))
            .await?;
        Ok(())
    }

    /// A `/command` that arrived while a turn runs, routed by [`busy_policy`]: run beside the turn,
    /// queue for the turn boundary (never into the steer inbox), or refuse.
    pub(super) async fn busy_command(&mut self, line: String) -> Result<()> {
        let autonomous = self.loop_state.is_some() || self.goal_state.is_some();
        match route_busy_line(&line, &self.catalog, autonomous) {
            BusyRoute::RunNow => Box::pin(self.submit_line(line, None)).await?,
            BusyRoute::Queue => {
                self.prompt_history.record(&line);
                queue_command(&mut self.queued_prompts, &mut self.app, &line);
            }
            BusyRoute::Refuse => self
                .app
                .note("⏳ commands run when the turn is idle — finish or Stop first"),
            BusyRoute::RefuseAutonomous => self.app.note(REFUSE_AUTONOMOUS_NOTE),
        }
        Ok(())
    }

    /// Run what the queue holds, in typed order, until a turn is running or the queue is empty.
    /// Queued commands go through normal idle dispatch.
    pub(super) async fn drain_queue(&mut self) {
        while self.turn_handle.is_none() {
            match dequeue_next(&mut self.queued_prompts, &mut self.app) {
                Some(Queued::Command(line)) => {
                    if let Err(e) = Box::pin(self.submit_line(line, None)).await {
                        self.app.note(&format!("⚠ {e}"));
                    }
                }
                Some(Queued::Prompt(next)) => self.start_turn(&next),
                None => break,
            }
        }
    }

    /// Submit one idle-state line: `//` escape, `/command` dispatch, or a plain prompt —
    /// the same routing the TUI's submit path applies. `explicit_mentions`, when `Some`, is the
    /// current `Prompt`'s own message-correlated non-image attachment list (mobile upload race
    /// fix) and is authoritative in the plain-prompt branch below; `None` (every other caller —
    /// the local key-driven submit paths, which never carry a fresh attachment list) falls back
    /// to exactly the old ambient `pending_mentions` behavior.
    pub(super) async fn submit_line(
        &mut self,
        line: String,
        explicit_mentions: Option<Vec<String>>,
    ) -> Result<()> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Ok(());
        }
        self.prompt_history.record(trimmed);
        if let Some(rest) = line.strip_prefix("//") {
            let (hooks, hook_workspace, hook_runner, hook_session) = {
                let mut s = self.session.lock().await;
                s.name_from_prompt(&line);
                (
                    s.hooks().to_vec(),
                    s.workspace_root().to_path_buf(),
                    s.llm_hook_runner().await,
                    s.id().to_string(),
                )
            };
            let escaped = format!("/{rest}");
            match forge_core::hooks_llm::with_runner(
                hook_runner,
                forge_core::hooks::run_prompt_hooks_for(
                    &hooks,
                    &escaped,
                    Some(&hook_workspace),
                    &hook_session,
                ),
            )
            .await
            {
                Err(reason) => self
                    .app
                    .note(&format!("⎇ prompt blocked by hook: {reason}")),
                Ok(prompt) => self.start_turn(&prompt),
            }
            return Ok(());
        }
        if line.starts_with('/') {
            let outcome = dispatch_command(
                &line,
                &self.session,
                None,
                &mut self.app,
                &self.catalog,
                &mut self.armed_project,
                self.trust_project,
                self.busy,
                &mut self.assay_lenses,
                &mut self.assay_scope,
            )
            .await?;
            self.handle_outcome(outcome);
            return Ok(());
        }
        // Uploaded text files ride this prompt as @path mentions — the explicit,
        // message-correlated list (if this prompt carried one) is authoritative; otherwise fall
        // back to exactly the old ambient `Attach`-then-`Prompt` behavior.
        let line = match explicit_mentions {
            Some(mut mentions) => prepend_attach_mentions(&mut mentions, line),
            None => prepend_attach_mentions(&mut self.pending_mentions, line),
        };
        let (hooks, hook_workspace, hook_runner, hook_session) = {
            let mut s = self.session.lock().await;
            s.name_from_prompt(&line);
            (
                s.hooks().to_vec(),
                s.workspace_root().to_path_buf(),
                s.llm_hook_runner().await,
                s.id().to_string(),
            )
        };
        match forge_core::hooks_llm::with_runner(
            hook_runner,
            forge_core::hooks::run_prompt_hooks_for(
                &hooks,
                &line,
                Some(&hook_workspace),
                &hook_session,
            ),
        )
        .await
        {
            Err(reason) => self
                .app
                .note(&format!("⎇ prompt blocked by hook: {reason}")),
            Ok(prompt) => {
                // Expand `@path` mentions exactly like the TUI submit path.
                attach_at_images(&prompt, Some(&hook_workspace), &self.session, &mut self.app)
                    .await;
                let (file_blocks, included, skipped) = expand_at_files_in(&prompt, &hook_workspace);
                if !included.is_empty() {
                    self.app
                        .note(&format!("📎 included {}", included.join(", ")));
                }
                for s in &skipped {
                    self.app.note(&format!("⚠ skipped {s}"));
                }
                self.last_prompt = Some(prompt.clone());
                if file_blocks.is_empty() {
                    self.start_turn(&prompt);
                } else {
                    self.turn_gen += 1;
                    self.turn_handle = Some(spawn_turn_with(
                        prompt.clone(),
                        file_blocks,
                        None,
                        &self.session,
                        &self.done_tx,
                        self.turn_gen,
                        &mut self.app,
                        &mut self.busy,
                        &mut self.busy_since,
                    ));
                }
            }
        }
        Ok(())
    }
}
