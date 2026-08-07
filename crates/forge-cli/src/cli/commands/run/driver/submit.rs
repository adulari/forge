//! What a remote client's input does to a daemon-hosted session.
//!
//! This is the headless mirror of the TUI's remote drain: the same routing for `//` escapes,
//! `/command` dispatch, and plain prompts, minus the cases that only mean something on a host
//! terminal. Two behaviours are load-bearing and live here because they are decided at input
//! time, not turn time: input that arrives while a turn is running is queued rather than dropped,
//! and a `Prompt`'s own message-correlated attachment list is authoritative for that turn (the
//! mobile upload race), overriding ambient state from an unrelated earlier `Attach`.

use super::*;

impl DriverState {
    /// One remote input — the headless mirror of `run_chat_tui`'s remote drain, minus the
    /// host-terminal cases (`/remote` toggling, host clipboard).
    pub(super) async fn handle_input(&mut self, input: remote::RemoteInput) -> Result<()> {
        match input {
            remote::RemoteInput::Prompt { text, attachments } => {
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
                    } else if trimmed.starts_with('/') && !trimmed.starts_with("//") {
                        self.app
                            .note("⏳ commands run when the turn is idle — finish or Stop first");
                    } else {
                        self.queued_prompts.push(text.clone());
                        self.app.set_queued(&self.queued_prompts);
                        self.app.note(&format!(
                            "⏳ queued ({} pending) — runs after this turn",
                            self.queued_prompts.len()
                        ));
                    }
                    return Ok(());
                }
                self.submit_line(text, has_explicit_attachments.then_some(explicit_mentions))
                    .await?;
            }
            remote::RemoteInput::Allow { yes, seq } => {
                if !remote::prompt_seq_current(self.prompt_seq, seq) {
                    push_remote_note(
                        &mut self.notes,
                        "⚠ stale answer ignored — the prompt changed; review the current one",
                    );
                } else if let Some((tool, reply)) = self.pending.take() {
                    let outcome = if yes {
                        ConfirmOutcome::Allow
                    } else {
                        ConfirmOutcome::Deny
                    };
                    let _ = reply.send(outcome);
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
                    if let Some(ans) = self.app.resolve_question(&text) {
                        if let Some(tx) = self.pending_question.take() {
                            let _ = tx.send(ans);
                        }
                    } else {
                        self.app.note("⚠ remote answer was invalid — re-asking");
                    }
                }
            }
            remote::RemoteInput::Interrupt => {
                if self.busy {
                    self.interrupt_turn();
                    self.app.note("⏹ remote interrupted — stopped responding");
                }
            }
            remote::RemoteInput::Dequeue { index, text } => {
                let idx = index as usize;
                if idx < self.queued_prompts.len() && self.queued_prompts[idx] == text {
                    self.queued_prompts.remove(idx);
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
            remote::RemoteInput::Steer { text } => {
                if self.busy {
                    forge_core::fleet::insert_into_queue(
                        &mut self.queued_prompts,
                        forge_core::fleet::MessageMode::Steer,
                        text,
                    );
                    self.app.set_queued(&self.queued_prompts);
                    self.app.note("⚡ steered — next up when this turn ends");
                } else {
                    self.submit_line(text, None).await?;
                }
            }
        }
        Ok(())
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
        if !trimmed.is_empty() && self.prompt_history.last().map(String::as_str) != Some(trimmed) {
            self.prompt_history.push(trimmed.to_string());
        }
        if let Some(rest) = line.strip_prefix("//") {
            let (hooks, hook_workspace) = {
                let s = self.session.lock().await;
                (s.hooks().to_vec(), s.workspace_root().to_path_buf())
            };
            let escaped = format!("/{rest}");
            match forge_core::hooks::run_prompt_hooks_in(&hooks, &escaped, Some(&hook_workspace))
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
        let (hooks, hook_workspace) = {
            let s = self.session.lock().await;
            (s.hooks().to_vec(), s.workspace_root().to_path_buf())
        };
        match forge_core::hooks::run_prompt_hooks_in(&hooks, &line, Some(&hook_workspace)).await {
            Err(reason) => self
                .app
                .note(&format!("⎇ prompt blocked by hook: {reason}")),
            Ok(prompt) => {
                // Expand `@path` mentions exactly like the TUI submit path.
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
