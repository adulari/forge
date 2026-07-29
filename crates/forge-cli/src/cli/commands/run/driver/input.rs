//! Headless remote key routing and asynchronous overlay completion.

use super::*;

impl DriverState {
    /// The headless key router: modal surfaces first (same precedence as the TUI key loop),
    /// then plain input editing. Only surfaces a remote client can actually drive are handled;
    /// host-terminal-only hotkeys have no headless meaning and are ignored.
    pub(super) async fn process_key(&mut self, key: KeyKind) -> Result<()> {
        // The workflow view is modal while open.
        if self.app.workflow.open {
            if self.app.workflow.zoom.is_some() {
                workflow_zoom_key(&mut self.app, key);
            } else {
                match key {
                    KeyKind::Esc | KeyKind::Char('q') => self.app.workflow.open = false,
                    KeyKind::Up | KeyKind::Char('k') => self.app.workflow.move_selection(-1),
                    KeyKind::Down | KeyKind::Char('j') => self.app.workflow.move_selection(1),
                    KeyKind::PageUp => self.app.workflow.move_selection(-5),
                    KeyKind::PageDown => self.app.workflow.move_selection(5),
                    KeyKind::Home => self.app.workflow.selected = 0,
                    KeyKind::End => {
                        self.app.workflow.selected = self.app.workflow.rows.len().saturating_sub(1);
                    }
                    KeyKind::Enter if !self.app.workflow.rows.is_empty() => {
                        self.app.workflow.zoom = Some(Default::default());
                    }
                    _ => {}
                }
            }
            return Ok(());
        }
        // The /config editor is modal while open — same actions as the TUI, except the
        // $EDITOR jump (host-terminal-only).
        if self.app.config_editor.open {
            match self.app.config_editor.handle_key(key) {
                forge_tui::ConfigAction::Save { path, value } => {
                    let result = if let Some(provider) = path.strip_prefix("key.") {
                        if value.trim().is_empty() {
                            forge_config::remove_api_key(provider)
                                .map(|_| ())
                                .map_err(|e| e.to_string())
                        } else {
                            forge_config::store_api_key(provider, value.trim())
                                .map_err(|e| e.to_string())
                        }
                    } else {
                        let scope = if self.app.config_editor.project_scope {
                            forge_config::ConfigScope::Project
                        } else {
                            forge_config::ConfigScope::User
                        };
                        forge_config::set_config_value(scope, &path, &value)
                            .map_err(|e| e.to_string())
                    };
                    match result {
                        Ok(()) => {
                            self.app.config_editor.rows = config_editor_rows();
                            self.app.config_editor.status = Some(format!("✓ saved {path}"));
                        }
                        Err(e) => self.app.config_editor.status = Some(format!("✗ {e}")),
                    }
                }
                forge_tui::ConfigAction::Reset { path } => {
                    let scope = if self.app.config_editor.project_scope {
                        forge_config::ConfigScope::Project
                    } else {
                        forge_config::ConfigScope::User
                    };
                    match forge_config::reset_config_value(scope, &path) {
                        Ok(()) => {
                            self.app.config_editor.rows = config_editor_rows();
                            self.app.config_editor.status =
                                Some(format!("✓ reset {path} to default"));
                        }
                        Err(e) => self.app.config_editor.status = Some(format!("✗ {e}")),
                    }
                }
                forge_tui::ConfigAction::Reload => {
                    self.app.config_editor.rows = config_editor_rows();
                }
                forge_tui::ConfigAction::EditFile => {
                    self.app.config_editor.status =
                        Some("⚠ editing this section needs $EDITOR on the host".to_string());
                }
                forge_tui::ConfigAction::Close | forge_tui::ConfigAction::None => {}
            }
            return Ok(());
        }
        // Effort slider (opened by bare /effort).
        if self.app.effort_slider {
            match key {
                KeyKind::Left | KeyKind::Right => {
                    if matches!(key, KeyKind::Left) {
                        self.app.effort_slider_left();
                    } else {
                        self.app.effort_slider_right();
                    }
                    if let Some(level) = self.app.effort {
                        if let Ok(mut s) = self.session.try_lock() {
                            s.set_effort(Some(level));
                        } else {
                            let s = self.session.clone();
                            tokio::spawn(async move { s.lock().await.set_effort(Some(level)) });
                        }
                    }
                }
                KeyKind::Esc | KeyKind::Enter | KeyKind::ToggleEffortSlider => {
                    self.app.effort_slider = false;
                }
                _ => {}
            }
            return Ok(());
        }
        // Command palette.
        if self.app.palette.open {
            match key {
                KeyKind::Esc => {
                    self.app.palette.close();
                    self.app.input.clear();
                }
                KeyKind::Up => self.app.palette.move_up(),
                KeyKind::Down => self.app.palette.move_down(),
                KeyKind::Tab => {
                    if let Some(name) = self.app.palette.selected_name().map(|s| s.to_string()) {
                        self.app.input = format!("/{name}");
                        self.app.input_cursor = self.app.input.len();
                        self.app.palette.query = name;
                        self.app.palette.clamp();
                    }
                }
                KeyKind::Enter => {
                    let leading =
                        self.app.input.starts_with('/') && !self.app.input.starts_with("//");
                    if !leading {
                        self.app.palette.close();
                        return Ok(());
                    }
                    let has_args = self.app.input.trim().contains(char::is_whitespace);
                    let line = if has_args {
                        self.app.input.clone()
                    } else {
                        self.app
                            .palette
                            .selected_name()
                            .map(|n| format!("/{n}"))
                            .unwrap_or_else(|| self.app.input.clone())
                    };
                    self.app.palette.close();
                    self.app.input.clear();
                    if self.busy {
                        self.app
                            .note("⏳ commands run when the turn is idle — finish or Stop first");
                    } else {
                        // Route through the shared submit path (identical DispatchOutcome
                        // handling to a directly-typed command).
                        Box::pin(self.submit_line(line, None)).await?;
                    }
                }
                _ => {
                    let _ = handle_key(&mut self.app.input, &mut self.app.input_cursor, key);
                    sync_palette_to_slash_token(&mut self.app);
                }
            }
            return Ok(());
        }
        // Usage overlay: informational, Esc closes.
        if self.app.usage_overlay.open {
            if matches!(key, KeyKind::Esc) {
                self.app.usage_overlay.open = false;
            }
            return Ok(());
        }
        // Mesh inspector overlay.
        if self.app.mesh_overlay.open {
            match key {
                KeyKind::Esc => {
                    self.app.mesh_overlay.open = false;
                    self.app.mesh_overlay.cursor = 0;
                }
                KeyKind::Down => {
                    let max = self.app.mesh_overlay.candidates.len().saturating_sub(1);
                    self.app.mesh_overlay.cursor = (self.app.mesh_overlay.cursor + 1).min(max);
                }
                KeyKind::Up => {
                    self.app.mesh_overlay.cursor = self.app.mesh_overlay.cursor.saturating_sub(1);
                }
                _ => {}
            }
            return Ok(());
        }
        // @path file picker.
        if self.app.at_picker.open {
            match key {
                KeyKind::Esc => self.app.at_picker.close(),
                KeyKind::Up => self.app.at_picker.move_up(),
                KeyKind::Down => self.app.at_picker.move_down(),
                KeyKind::Tab | KeyKind::Enter => {
                    if let Some(path) = self.app.at_picker.selected_path() {
                        if let Some(tok) = forge_tui::at_token_at(
                            &self.app.input,
                            self.app.input_cursor.min(self.app.input.len()),
                        ) {
                            self.app
                                .input
                                .replace_range(tok.start..tok.end, &format!("@{path} "));
                            self.app.input_cursor = self.app.input.len();
                        } else {
                            self.app.input = format!("@{path} ");
                            self.app.input_cursor = self.app.input.len();
                        }
                    }
                    self.app.at_picker.close();
                }
                KeyKind::Char(c) => {
                    self.app.input.push(c);
                    sync_at_picker_to_at_token(&mut self.app);
                }
                KeyKind::Backspace => {
                    self.app.input.pop();
                    sync_at_picker_to_at_token(&mut self.app);
                }
                _ => {}
            }
            return Ok(());
        }
        // The generic picker (sessions / checkpoints / models / tempers / assay / copy / duel…).
        if self.app.picker.open {
            match key {
                KeyKind::Esc => {
                    if self.app.picker.kind == Some(forge_tui::PickerKind::Models)
                        && self.app.models_drilled.is_some()
                    {
                        open_models_root(&self.session, &mut self.app).await?;
                    } else {
                        self.app.models_drilled = None;
                        self.app.models_pin_mode = false;
                        if self.app.picker.kind == Some(forge_tui::PickerKind::Duel) {
                            self.duel_state = None;
                            self.app.note("⚔ duel discarded — no candidate was merged");
                        }
                        self.app.picker.close();
                    }
                }
                KeyKind::Up => self.app.picker.move_up(),
                KeyKind::Down => self.app.picker.move_down(),
                KeyKind::Tab if self.app.picker.kind == Some(forge_tui::PickerKind::Sessions) => {
                    let query = self.app.picker.query.clone();
                    self.app.show_archived = !self.app.show_archived;
                    open_sessions_picker(&mut self.app, &query)?;
                }
                KeyKind::DeleteForward
                    if self.app.picker.kind == Some(forge_tui::PickerKind::Sessions) =>
                {
                    if let Some(row) = self.app.picker.selected_row() {
                        if !row.id.starts_with("observe:") {
                            let store = crate::open_store()?;
                            if self.app.show_archived {
                                store.unarchive_session(&row.id)?;
                            } else {
                                store.archive_session(&row.id)?;
                            }
                            let query = self.app.picker.query.clone();
                            open_sessions_picker(&mut self.app, &query)?;
                        }
                    }
                }
                KeyKind::Enter => {
                    self.picker_enter().await?;
                }
                KeyKind::Char(c) => {
                    self.app.picker.query.push(c);
                    self.app.picker.clamp();
                }
                KeyKind::Backspace => {
                    self.app.picker.query.pop();
                    self.app.picker.clamp();
                }
                _ => {}
            }
            return Ok(());
        }
        // Esc: interrupt a running turn; idle Esc was already filtered at the drain.
        if matches!(key, KeyKind::Esc) {
            if self.busy {
                self.interrupt_turn();
                self.app.note("⏹ interrupted — stopped responding");
            }
            return Ok(());
        }
        // Temper cycling (SHIFT+TAB on the page).
        if matches!(key, KeyKind::CycleTemper | KeyKind::TemperCycle) {
            let Some(new) = self
                .session
                .try_lock()
                .ok()
                .map(|mut sess| sess.cycle_temper())
            else {
                self.app.note("⚠ try again in a moment — session is busy");
                return Ok(());
            };
            self.app.set_temper(new.label());
            let _ = forge_config::write_permission_mode(new);
            return Ok(());
        }
        // Plain input editing: keys accumulate into the input line; Enter submits (queued if
        // busy, exactly like local typing mid-turn).
        let outcome = handle_key(&mut self.app.input, &mut self.app.input_cursor, key);
        if let InputOutcome::Submit(raw_line) = outcome {
            let (line, _imgs) = self.app.resolve_paste_blocks(raw_line);
            if self.busy {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    // nothing to queue
                } else if trimmed.starts_with('/') && !trimmed.starts_with("//") {
                    self.app
                        .note("⏳ commands run when the turn is idle — finish or Stop first");
                } else {
                    self.queued_prompts.push(line);
                    self.app.set_queued(&self.queued_prompts);
                }
            } else {
                Box::pin(self.submit_line(line, None)).await?;
            }
        } else {
            // Keep the palette/@-picker in sync with what the (remote) cursor sits in, same as
            // the TUI's editing branch.
            let cur = self.app.input_cursor.min(self.app.input.len());
            let tok = forge_tui::slash_token_at(&self.app.input, cur)
                .filter(|t| cur >= t.start && cur <= t.end);
            if let Some(tok) = tok {
                self.app.at_picker.close();
                self.app.palette.open_with(&tok.name);
            } else {
                self.app.palette.close();
                sync_at_picker_to_at_token(&mut self.app);
            }
        }
        Ok(())
    }

    /// Enter on the generic picker — the headless mirror of the TUI's picker-Enter branch.
    async fn picker_enter(&mut self) -> Result<()> {
        let chosen = self.app.picker.selected_row().cloned();
        let kind = self.app.picker.kind;
        if kind == Some(forge_tui::PickerKind::Models) {
            if let Some(row) = chosen {
                if self.app.models_drilled.is_none() && !row.id.contains("::") {
                    open_models_provider(&self.session, &mut self.app, &row.id).await?;
                } else if row.id.contains("::") && self.app.models_pin_mode {
                    let model_id = forge_provider::normalize_model_id(&row.id).into_owned();
                    if let Ok(mut s) = self.session.try_lock() {
                        s.pin_model(Some(model_id.clone()));
                    } else {
                        let s = self.session.clone();
                        let m = model_id.clone();
                        tokio::spawn(async move { s.lock().await.pin_model(Some(m)) });
                    }
                    self.app.models_pin_mode = false;
                    self.app.models_drilled = None;
                    self.app.picker.close();
                    self.app
                        .note(&format!("⊕ model pinned: {model_id} (clears with /model)"));
                }
            }
            return Ok(());
        }
        self.app.picker.close();
        let (Some(row), Some(kind)) = (chosen, kind) else {
            return Ok(());
        };
        if kind == forge_tui::PickerKind::AssayChoice {
            self.turn_gen += 1;
            let lenses = std::mem::take(&mut self.assay_lenses);
            let scope = std::mem::replace(&mut self.assay_scope, forge_types::AssayScope::Repo);
            self.turn_handle = spawn_assay(
                row.id == "cleanup",
                lenses,
                scope,
                &self.session,
                &self.done_tx,
                self.turn_gen,
                &mut self.app,
                &mut self.busy,
                &mut self.busy_since,
            )
            .await?;
        } else if kind == forge_tui::PickerKind::CopyBlocks {
            if let Some((_, text)) = row
                .id
                .parse::<usize>()
                .ok()
                .and_then(|i| self.app.copy_candidates.get(i).cloned())
            {
                let chars = text.chars().count();
                push_remote_note(
                    &mut self.notes,
                    &format!("✓ copy ready ({chars} chars) — tap “Copy here”"),
                );
                self.copy_text = Some(text);
            }
            self.app.copy_candidates.clear();
        } else if kind == forge_tui::PickerKind::Duel {
            if let Some((report, guards)) = self.duel_state.take() {
                let repo_root = std::path::PathBuf::from(&self.cwd);
                let repo_key = repo_root.display().to_string();
                let winner_branch = row.id.clone();
                let merge_note = match forge_core::duel::merge_winner(&repo_root, &winner_branch) {
                    Ok(m) if m.conflicted_files.is_empty() => "merged cleanly".to_string(),
                    Ok(m) => format!(
                        "merged with conflicts in: {}",
                        m.conflicted_files.join(", ")
                    ),
                    Err(e) => format!("merge failed: {e}"),
                };
                if let Ok(store) = crate::open_store() {
                    for c in &report.candidates {
                        let won = c.branch == winner_branch;
                        let _ = store.record_duel_outcome(&repo_key, &c.model, won, &report.task);
                    }
                }
                let winner_model = report
                    .candidates
                    .iter()
                    .find(|c| c.branch == winner_branch)
                    .map(|c| c.model.clone())
                    .unwrap_or_else(|| "?".to_string());
                self.app
                    .note(&format!("⚔ duel winner: {winner_model} — {merge_note}"));
                drop(guards);
            }
        } else if kind == forge_tui::PickerKind::Sessions && row.id.starts_with("observe:") {
            self.app
                .note("⚠ observing a live MCP session isn't available from the daemon page");
        } else {
            picker_accept(kind, &row, &self.session, None, &mut self.app).await?;
        }
        Ok(())
    }

    /// A turn-done signal: mirrors the TUI's done drain — duel picker, /loop continuation,
    /// queued prompts, auto-compact.
    pub(super) async fn on_turn_done(&mut self, g: u64) {
        if !(self.busy && g == self.turn_gen) {
            return;
        }
        self.busy = false;
        self.turn_handle = None;
        if let Some(json) = self.app.view_snapshot_json() {
            self.session.lock().await.save_view_snapshot(&json);
        }
        if let Some((report, guards)) = self.pending_duel.lock().unwrap().take() {
            if report.candidates.is_empty() {
                self.app.note("⚔ duel produced no usable candidates");
            } else {
                let rows = duel_picker_rows(&report);
                self.app.picker.open_with(
                    forge_tui::PickerKind::Duel,
                    &format!("⚔ duel — pick the winner ({} candidates)", rows.len()),
                    rows,
                );
                self.duel_state = Some((report, guards));
            }
        }
        if let Some(ls) = self.loop_state.take() {
            if ls.gen == g {
                let last = {
                    self.session
                        .lock()
                        .await
                        .last_assistant_text()
                        .map(str::to_string)
                };
                match loop_stop_reason(last.as_deref(), ls.iter) {
                    Some(reason) => self.app.note(reason),
                    None => {
                        let prompt = self
                            .take_next_queued_prompt()
                            .unwrap_or_else(|| "Continue toward completion.".to_string());
                        self.last_prompt = Some(prompt.clone());
                        self.turn_gen += 1;
                        self.loop_state = Some(LoopState {
                            gen: self.turn_gen,
                            iter: ls.iter + 1,
                        });
                        self.turn_handle = Some(spawn_turn_with(
                            prompt,
                            vec![LOOP_GUIDANCE.to_string()],
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
            } else {
                self.loop_state = Some(ls);
            }
        }
        if let Some(gs) = self.goal_state.take() {
            if gs.gen == g {
                let (done, total) = {
                    let s = self.session.lock().await;
                    let tasks = s.tasks();
                    (
                        tasks
                            .iter()
                            .filter(|t| t.status == forge_types::TodoStatus::Done)
                            .count(),
                        tasks.len(),
                    )
                };
                let last = {
                    self.session
                        .lock()
                        .await
                        .last_assistant_text()
                        .map(str::to_string)
                };
                let said_complete = is_goal_complete_marker(last.as_deref());
                let progressed = done > gs.prev_done;
                let no_progress = if progressed { 0 } else { gs.no_progress + 1 };
                match goal_stop_reason(said_complete, done, total, gs.iter, no_progress) {
                    Some(reason) if is_goal_complete_reason(reason) => {}
                    Some(reason) => self.app.note(reason),
                    None => {
                        let prompt = self
                            .take_next_queued_prompt()
                            .unwrap_or_else(|| GOAL_CONTINUE_PROMPT.to_string());
                        self.last_prompt = Some(prompt.clone());
                        self.turn_gen += 1;
                        self.goal_state = Some(GoalState {
                            gen: self.turn_gen,
                            iter: gs.iter + 1,
                            prev_done: done,
                            no_progress,
                            goal: gs.goal,
                        });
                        self.turn_handle = Some(spawn_turn_with(
                            prompt,
                            vec![GOAL_GUIDANCE.to_string()],
                            Some(forge_types::TaskTier::Complex),
                            &self.session,
                            &self.done_tx,
                            self.turn_gen,
                            &mut self.app,
                            &mut self.busy,
                            &mut self.busy_since,
                        ));
                    }
                }
            } else {
                self.goal_state = Some(gs);
            }
        }
        if self.turn_handle.is_none() {
            if let Some(next) = self.take_next_queued_prompt() {
                self.start_turn(&next);
            }
        }
        if self.turn_handle.is_none() && self.turn_gen > self.last_auto_compact_gen {
            if let Some(lim) = self.app.context_limit {
                let cap = self.session.lock().await.compact_cap_tokens();
                let trigger = forge_core::auto_compact_trigger_tokens(
                    lim as u64,
                    cap,
                    AUTO_COMPACT_THRESHOLD,
                );
                if self.app.context_tokens > trigger {
                    let fill = self.app.context_tokens as f64 / lim as f64;
                    self.app.note(&format!(
                        "⚒ context {:.0}% full — auto-compacting",
                        fill * 100.0
                    ));
                    self.turn_gen += 1;
                    self.last_auto_compact_gen = self.turn_gen;
                    self.turn_handle = Some(spawn_compact(
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
    }

    /// Poll the background overlay loads (/mesh, /usage) — same as the TUI's per-frame polls.
    /// Returns true when a load resolved: the idle loop only broadcasts snapshots on dirty
    /// frames, so an unreported resolution here would leave every remote client staring at the
    /// loading spinner until some unrelated input arrived.
    pub(super) fn poll_overlay_loads(&mut self) -> bool {
        let mut changed = false;
        if let Some(rx) = &mut self.mesh_load_rx {
            match rx.try_recv() {
                Ok(Some(overlay)) => {
                    let tick = self.app.mesh_overlay.anim_tick;
                    self.app.mesh_overlay = overlay;
                    self.app.mesh_overlay.anim_tick = tick;
                    self.mesh_load_rx = None;
                    changed = true;
                }
                Ok(None) => {
                    self.app.mesh_overlay.open = false;
                    self.mesh_load_rx = None;
                    self.app.push_scrollback_text(
                        "mesh: auto-discovery routing is off (no model catalog) — nothing to inspect",
                    );
                    changed = true;
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    self.app.mesh_overlay.open = false;
                    self.mesh_load_rx = None;
                    changed = true;
                }
            }
        }
        if let Some(rx) = &mut self.usage_load_rx {
            match rx.try_recv() {
                Ok(bstats) => {
                    let (fracs, claude_store_age_secs) = self
                        .session
                        .try_lock()
                        .map(|s| {
                            seed_subscription_stats(&s, &bstats);
                            (s.bridge_fractions(), s.claude_quota_age_secs())
                        })
                        .unwrap_or_default();
                    self.app.usage_overlay.claude_5h_in = bstats.claude_5h_in;
                    self.app.usage_overlay.claude_5h_out = bstats.claude_5h_out;
                    self.app.usage_overlay.claude_weekly_in = bstats.claude_weekly_in;
                    self.app.usage_overlay.claude_weekly_out = bstats.claude_weekly_out;
                    fill_subscription_pcts(
                        &mut self.app.usage_overlay,
                        &fracs,
                        claude_store_age_secs,
                    );
                    self.app.usage_overlay.loading = false;
                    self.usage_load_rx = None;
                    changed = true;
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    self.app.usage_overlay.loading = false;
                    self.usage_load_rx = None;
                    changed = true;
                }
            }
        }
        changed
    }
}
