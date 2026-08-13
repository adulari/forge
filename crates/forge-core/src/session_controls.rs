//! Session controls, surface integration, pricing, quota, and runtime attachments.
//!
//! This owner retains all direct state mutations that configure a live Session.

use super::*;

impl Session {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn workspace_root(&self) -> &std::path::Path {
        self.workspace.root()
    }

    pub fn workspace_binding(&self) -> Arc<std::sync::RwLock<std::path::PathBuf>> {
        Arc::clone(&self.workspace_binding)
    }

    pub fn workspace_scope(&self) -> String {
        self.workspace.display()
    }

    pub fn lattice_root(&self) -> Option<&str> {
        self.lattice.as_deref().map(Lattice::repo_root)
    }

    pub fn cached_agents_md(&self) -> Option<&str> {
        self.cached_agents_md.as_deref()
    }

    /// The ordered context that Forge injected during the most recent turn.
    pub fn last_context_pack(&self) -> &context_pack::ContextPack {
        &self.last_context_pack
    }

    /// The completion expectation Forge applied to the most recent turn.
    pub fn last_turn_contract(&self) -> &turn_contract::TurnContract {
        &self.last_turn_contract
    }

    /// Persist a system-context message and add its provenance to the active turn's audit pack.
    pub(crate) fn inject_context(
        &mut self,
        pack: &mut context_pack::ContextPack,
        source: context_pack::ContextSource,
        reason: &str,
        content: &str,
    ) -> Result<(), CoreError> {
        let seq = self.next_seq();
        self.store
            .add_message(&self.id, seq, Role::System, content, None)?;
        self.transcript.push(Message::system(content));
        pack.push(source, reason, content);
        Ok(())
    }

    /// Publish the one accepted terminal answer. Provider-visible provisional completions stay in
    /// the lossless transcript as `LlmOnly`; this UI-only copy is the sole conversation answer.
    pub(crate) fn publish_terminal_answer(&mut self, content: &str) -> Result<(), CoreError> {
        if content.trim().is_empty() {
            return Ok(());
        }
        let seq = self.next_seq();
        self.store
            .add_ui_note(&self.id, seq, Role::Assistant, content)?;
        self.transcript.push(Message::assistant(content).ui_only());
        self.presenter.emit(PresenterEvent::AssistantDone);
        Ok(())
    }

    /// Queue images to attach to the next user turn (vision input). Consumed when that turn's user
    /// message is built; a turn with no images behaves exactly as before.
    pub fn attach_images(&mut self, images: Vec<forge_types::ImageAttachment>) {
        self.pending_images.extend(images);
    }

    /// Discard whatever's queued for the next turn's vision input WITHOUT using it — the
    /// counterpart to [`Session::attach_images`]. Used when an explicit, message-correlated
    /// attachment list has arrived for a turn and any stale ambient state from an unrelated
    /// upload must not leak into it (or any future turn).
    pub fn take_pending_images(&mut self) -> Vec<forge_types::ImageAttachment> {
        std::mem::take(&mut self.pending_images)
    }

    /// Whether project-scope (`./.forge/`) commands/skills run without a first-use confirmation.
    pub fn commands_trust_project(&self) -> bool {
        self.config.commands.trust_project
    }

    /// Attach the discovered catalog so the `/models` browser can read it (composition root).
    pub fn set_catalog(&mut self, catalog: Option<ModelCatalog>) {
        let calibration = self
            .store
            .model_outcome_calibration()
            .unwrap_or_default()
            .into_iter()
            .map(|row| {
                (
                    row.model,
                    forge_mesh::RuntimeCalibration {
                        samples: row.samples,
                        success_rate: row.success_rate,
                        mean_latency_ms: row.mean_latency_ms,
                    },
                )
            })
            .collect();
        self.catalog = catalog.map(|catalog| catalog.with_runtime_calibration(calibration));
    }

    /// Pin (or clear) the in-session model override. When `Some`, subsequent turns use this model
    /// set instead of the mesh-routed pick. `None` returns to normal mesh routing. The list is
    /// passed through `parse_pin_set` so a comma-separated `/model a,b,c` / pre-parsed set and a
    /// single `/model a` share one representation.
    pub fn pin_model(&mut self, model_id: Option<String>) {
        self.pinned_model = model_id
            .map(|s| forge_mesh::parse_pin_set(&s))
            .filter(|set| !set.is_empty());
    }

    /// The currently-pinned model set, if any (`/model <id>` was called this session).
    pub fn pinned_model(&self) -> Option<&[String]> {
        self.pinned_model.as_deref()
    }

    /// Mark this session as a headless code-change run (`bench swe`): every prompt is known to
    /// demand an implementation, arming the empty-diff completion nudge (`mesh.nudge_empty_diff`).
    pub fn set_expect_code_change(&mut self, v: bool) {
        self.expect_code_change = v;
    }

    /// Whether the last [`Session::run_turn`] was classified TOOLS-UNAVAILABLE (wave 7): an
    /// `expect_code_change` CLI-bridge turn whose `mcp-serve` tool server failed to start, so it
    /// ran with no write tools and left an empty tree. The harness (`bench swe` / headless) reads
    /// this to retry the instance on a fresh bridge process rather than record a silent toolless
    /// run as a clean empty completion. Always false on interactive / direct-API sessions.
    pub fn tools_unavailable(&self) -> bool {
        self.tools_unavailable_run
    }

    /// Set the soft turn deadline (see the `turn_deadline` field): the caller enforces a hard
    /// timeout the session cannot see, so this is set to `hard limit − reserve` (the reserve
    /// leaves room for the one reconciliation turn). Re-arms the one-shot latch.
    pub fn set_turn_deadline(&mut self, deadline: std::time::Instant) {
        self.turn_deadline = Some(deadline);
        self.deadline_reconciled = false;
    }

    /// Whether the soft turn deadline is set, active (`mesh.deadline_reconcile`), and past.
    pub(crate) fn past_turn_deadline(&self) -> bool {
        self.config.mesh.deadline_reconcile
            && self
                .turn_deadline
                .is_some_and(|d| std::time::Instant::now() >= d)
    }

    /// Set (or clear) the in-session reasoning-effort pin. `None` returns to the provider default.
    pub fn set_effort(&mut self, e: Option<EffortLevel>) {
        if e != self.pinned_effort {
            // Entering (or re-entering) white-hot re-arms its one-shot guidance injection.
            self.whitehot_guidance_injected = false;
        }
        self.pinned_effort = e;
        // Persisted here rather than at the call sites so every path that changes effort — the
        // TUI slider, `/effort`, a remote client — is covered. Without it a resumed session
        // silently drops back to the provider default, which reads as the agent quietly
        // downgrading itself. Best-effort: a store failure must not fail the control.
        let _ = self
            .store
            .set_session_pinned_effort(&self.id, e.map(|level| level.as_str()));
    }

    /// The currently-pinned effort level, if any (`/effort <level>` was called this session).
    pub fn pinned_effort(&self) -> Option<EffortLevel> {
        self.pinned_effort
    }

    /// The currently-pinned routing tier, if any (set by `tier_up`/`tier_down`). `None` = normal
    /// mesh classification.
    pub fn pinned_tier(&self) -> Option<TaskTier> {
        self.pinned_tier
    }

    /// Set (or clear) the in-session routing-tier override. `None` returns to normal classification.
    pub fn pin_tier(&mut self, tier: Option<TaskTier>) {
        self.pinned_tier = tier;
    }

    /// Shift the routing-tier bias one step up (`up=true`) or down. The baseline is the current
    /// pin, or — when nothing is pinned yet — `from`, the last classified/displayed tier (so the
    /// first press moves relative to what the mesh would pick, not from a fixed middle). Clamped at
    /// the ends. Returns the new pinned tier so the caller can show a note.
    pub fn bump_tier(&mut self, up: bool, from: TaskTier) -> TaskTier {
        let base = self.pinned_tier.unwrap_or(from);
        let next = if up { base.up() } else { base.down() };
        self.pinned_tier = Some(next);
        next
    }

    /// The discovered model catalog, if auto-discovery ran for this session.
    pub fn catalog(&self) -> Option<&ModelCatalog> {
        self.catalog.as_ref()
    }

    /// Override the session's permission mode at runtime. Used by `forge mcp agent` so the
    /// orchestrating agent can switch to bypass/accept-edits without restarting the session.
    pub fn set_mode(&mut self, mode: PermissionMode) {
        self.mode = mode;
        self.config.permission_mode = mode;
    }

    /// The session's current permission mode.
    pub fn mode(&self) -> PermissionMode {
        self.mode
    }

    /// Attach connected MCP servers (composition root). Their tools become advertisable via
    /// `tool_specs` and callable through `invoke_tool`, gated by the permission broker.
    pub fn set_mcp(&mut self, mcp: Option<Arc<forge_mcp::McpManager>>) {
        // An empty manager (no servers connected) adds nothing — keep it `None` so the path stays
        // fully inert and `tool_specs` is byte-for-byte unchanged.
        self.mcp = mcp.filter(|m| !m.is_empty());
    }

    /// Attach the code-intelligence index (composition root). When set and `lattice.inject` is on,
    /// each turn auto-injects relevant code; the agent's edits reindex the touched file in-turn.
    pub fn set_lattice(&mut self, lattice: Option<Arc<Lattice>>) {
        self.lattice = lattice;
    }

    /// Attach the background reindex watcher's delivery channel (composition root). The watcher is
    /// built off-thread and sent through `rx`; holding the `Receiver` keeps it alive for the
    /// session's lifetime without ever blocking on its (possibly slow) setup.
    pub fn set_lattice_watcher(
        &mut self,
        rx: Option<std::sync::mpsc::Receiver<Result<forge_index::LatticeWatcher, String>>>,
    ) {
        self.lattice_watch_enabled = rx.is_some();
        self.lattice_watcher = rx;
    }

    /// Attach the background initial-index result channel. The update itself is deliberately
    /// detached from startup; this channel lets the next user turn report a failure visibly.
    pub fn set_lattice_update(
        &mut self,
        rx: Option<std::sync::mpsc::Receiver<Result<(), String>>>,
    ) {
        self.lattice_update = rx;
    }

    /// Recreate the background lattice watcher for the current workspace without blocking the
    /// caller on filesystem watcher setup.
    pub fn install_lattice_watcher(&mut self) {
        let Some(lattice) = self.lattice.as_ref().map(Arc::clone) else {
            return;
        };
        let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
        let Some(root) = forge_index::resolve_watch_root(self.workspace.root(), home.as_deref())
        else {
            self.lattice_watch_enabled = false;
            return;
        };
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result =
                forge_index::spawn_watcher(lattice, &root, std::time::Duration::from_millis(400));
            let _ = tx.send(result);
        });
        self.lattice_watch_enabled = true;
        self.lattice_watcher = Some(rx);
    }

    /// Surface detached Lattice setup failures without making the startup path synchronous.
    /// Successful watcher setup is retained here so dropping the session still stops its worker.
    pub(crate) fn poll_lattice_background(&mut self) {
        if let Some(rx) = self.lattice_update.take() {
            match rx.try_recv() {
                Ok(Ok(())) => {}
                Ok(Err(error)) => self.presenter.emit(forge_types::PresenterEvent::Warning(
                    format!("Lattice auto-index unavailable: {error}; code retrieval may be stale"),
                )),
                Err(std::sync::mpsc::TryRecvError::Empty) => self.lattice_update = Some(rx),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.presenter.emit(
                    forge_types::PresenterEvent::Warning(
                        "Lattice auto-index stopped before reporting a result; code retrieval may be stale"
                            .to_string(),
                    ),
                ),
            }
        }

        if let Some(rx) = self.lattice_watcher.take() {
            match rx.try_recv() {
                Ok(Ok(watcher)) => self.lattice_watcher_handle = Some(watcher),
                Ok(Err(error)) => {
                    self.lattice_watch_enabled = false;
                    self.presenter.emit(forge_types::PresenterEvent::Warning(format!(
                        "Lattice file watching unavailable: {error}; run `forge lattice update` after edits"
                    )));
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => self.lattice_watcher = Some(rx),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.lattice_watch_enabled = false;
                    self.presenter.emit(forge_types::PresenterEvent::Warning(
                        "Lattice file watching stopped before setup completed; run `forge lattice update` after edits"
                            .to_string(),
                    ));
                }
            }
        }
    }

    /// Attach the LSP registry (composition root). No-op when `lsp.enabled = false`.
    pub fn set_lsp(&mut self, lsp: Option<Arc<forge_lsp::LspRegistry>>) {
        self.lsp = lsp;
    }

    /// Attach the command/skill catalog (composition root) so the model can discover and load
    /// Forge's own skills via the `use_skill` tool. `None` (or an empty catalog) → not advertised.
    pub fn set_skills(&mut self, skills: Option<Arc<forge_skills::Catalog>>) {
        self.skills = skills;
    }

    pub fn skills(&self) -> Option<&Arc<forge_skills::Catalog>> {
        self.skills.as_ref()
    }

    /// Attach the fleet-messaging capability (composition root — `forge serve`'s daemon driver
    /// wires this in). `None` leaves `message_session` unadvertised, exactly like an unset `mcp`.
    pub fn set_fleet_messaging(&mut self, fleet: Option<Arc<dyn crate::fleet::FleetMessaging>>) {
        self.fleet = fleet;
    }

    /// Scoped subgraph for `symbol` from the session's live index (the `/lattice` view). `Ok(None)`
    /// when no index is attached.
    pub fn lattice_view(
        &self,
        symbol: &str,
    ) -> Result<Option<forge_index::LatticeView>, CoreError> {
        match &self.lattice {
            Some(l) => Ok(Some(l.view(symbol)?)),
            None => Ok(None),
        }
    }

    /// Per-server MCP status for the `/mcp` listing (empty when no servers are configured).
    pub fn mcp_status(&self) -> Vec<forge_types::McpServerLine> {
        self.mcp
            .as_ref()
            .map(|m| m.status_lines())
            .unwrap_or_default()
    }

    /// Emit the current MCP server listing to the presenter (called once at startup so connection
    /// status — including any failures — is visible). No-op when no servers are configured.
    pub fn announce_mcp(&mut self) {
        if self.mcp.is_some() {
            let lines = self.mcp_status();
            self.presenter.emit(PresenterEvent::McpStatus(lines));
        }
    }

    /// Subscribe to the MCP initial-connect completion signal. Returns `None` when no MCP servers
    /// are configured. The returned receiver holds `false` until all servers have resolved
    /// (connected or failed); then it's set to `true`. Use this to schedule a re-announce.
    pub fn mcp_connect_done(&self) -> Option<tokio::sync::watch::Receiver<bool>> {
        self.mcp.as_ref().map(|m| m.subscribe_done())
    }

    /// Connect a new MCP server into the live session. Creates the manager if none exists yet
    /// (e.g. the session was started with no MCP servers configured).
    pub async fn add_mcp_server(
        &mut self,
        server: forge_config::McpServerConfig,
    ) -> Result<(), CoreError> {
        match &self.mcp {
            Some(mgr) => mgr
                .connect_one(&server)
                .await
                .map_err(CoreError::Internal)?,
            None => {
                let mut cfg = forge_config::McpConfig::default();
                cfg.servers.push(server);
                let mgr = forge_mcp::McpManager::connect_all(&cfg).await;
                self.mcp = Some(Arc::new(mgr));
            }
        }
        Ok(())
    }

    /// Remove an MCP server from the live session by name. No-op if not connected.
    pub fn remove_mcp_server(&self, name: &str) {
        if let Some(mgr) = &self.mcp {
            mgr.disconnect(name);
        }
    }

    /// The full discovered tool list for one MCP server (`forge mcp --tools <server>`).
    pub fn mcp_tool_lines(&self, server: &str) -> Vec<(String, String)> {
        self.mcp
            .as_ref()
            .map(|m| m.tool_lines(server))
            .unwrap_or_default()
    }

    /// The pricing table in effect (bundled defaults + config overrides), for cost display.
    pub fn pricing(&self) -> &Pricing {
        &self.pricing
    }

    pub fn checkpoint_root(&self) -> &std::path::Path {
        &self.checkpoint_root
    }

    /// Override where code shadow-snapshots are stored (default `.forge/checkpoints`). Used by the
    /// composition root to anchor them under the project `.forge/`, and by tests for isolation.
    pub fn set_checkpoint_root(&mut self, root: impl Into<std::path::PathBuf>) {
        let root = root.into();
        self.checkpoint_root = if root.is_absolute() {
            root
        } else {
            self.workspace.root().join(root)
        };
        self.checkpoint_root_custom = true;
    }

    /// The session's current temper (permission mode).
    pub fn temper(&self) -> PermissionMode {
        self.mode
    }

    /// The hooks configured for this session. Used by the chat loop to fire lifecycle events
    /// (`UserPromptSubmit`, `SessionStart`, `SessionEnd`) outside the tool path.
    pub fn hooks(&self) -> &[forge_config::HookConfig] {
        &self.config.hooks
    }

    pub fn compact_cap_tokens(&self) -> u64 {
        self.config.mesh.compact_cap_tokens
    }

    /// The session id — used by lifecycle hooks to identify the session.
    pub fn session_id(&self) -> &str {
        &self.id
    }

    /// Fire the Claude-Code lifecycle hooks (`notification`, `pre_compact`, `post_compact`, `stop`,
    /// `subagent_stop`) for `event`, surfacing any output as a warning note. Inert (no spawn) when
    /// no hooks are configured, so it's safe to call on hot paths. `fields` are merged into the
    /// hook's stdin payload. Returns the [`hooks::LifecycleOutcome`] so a caller that enforces a
    /// block decision (`stop`/`subagent_stop`) can read `outcome.blocked`; observe-only callers
    /// (`notification`/`pre_compact`/`post_compact`) ignore the return.
    pub(crate) async fn fire_lifecycle(
        &mut self,
        event: forge_config::HookEvent,
        fields: serde_json::Value,
    ) -> hooks::LifecycleOutcome {
        if self.config.hooks.is_empty() {
            return hooks::LifecycleOutcome::default();
        }
        let fields = match fields {
            serde_json::Value::Object(mut fields) => {
                fields.insert("cwd".to_string(), self.workspace.display().into());
                serde_json::Value::Object(fields)
            }
            other => serde_json::json!({ "cwd": self.workspace.display(), "fields": other }),
        };
        let outcome = hooks::run_lifecycle_hooks(&self.config.hooks, event, &self.id, fields).await;
        for n in &outcome.notes {
            self.presenter.emit(PresenterEvent::Warning(n.clone()));
        }
        outcome
    }

    /// Persist the TUI view snapshot (opaque JSON) for this session so a resume restores the
    /// on-screen activity/viewer state. Best-effort — a store error is ignored.
    pub fn save_view_snapshot(&self, json: &str) {
        let _ = self.store.update_session_view_snapshot(&self.id, json);
    }

    /// The TUI view snapshot persisted for this session, if any (set on the last turn).
    pub fn view_snapshot(&self) -> Option<String> {
        self.store.session_view_snapshot(&self.id).ok().flatten()
    }

    /// The most recent assistant message's text, if any — used by `/loop` to decide whether the
    /// model signalled completion.
    pub fn last_assistant_text(&self) -> Option<&str> {
        self.transcript
            .iter()
            .rev()
            .find(|m| m.role == Role::Assistant)
            .map(|m| m.content.as_str())
    }

    /// Total spend today (UTC calendar day) across all sessions — the same figure the budget
    /// gate checks. Returns 0.0 on store error.
    pub fn spend_today_usd(&self) -> f64 {
        self.store.spend_today_usd().unwrap_or(0.0)
    }

    /// Total spend this month across all sessions. Returns 0.0 on store error.
    pub fn spend_this_month_usd(&self) -> f64 {
        self.store.spend_this_month_usd().unwrap_or(0.0)
    }

    /// Token and cost totals for the current session from the DB (reliable for bridge providers).
    pub fn session_usage_db(&self) -> (u64, u64, f64) {
        let id = self.session_id();
        let usage = self.store.session_token_usage(id).unwrap_or_default();
        let cost = self.store.session_cost(id).unwrap_or(0.0);
        (usage.input_tokens, usage.output_tokens, cost)
    }

    /// Spend in the last 5 hours (rolling window). Returns 0.0 on store error.
    pub fn spend_last_5h_usd(&self) -> f64 {
        self.store.spend_last_5h_usd().unwrap_or(0.0)
    }

    /// Spend in the current ISO week (Monday 00:00 local → now). Returns 0.0 on store error.
    pub fn spend_this_week_usd(&self) -> f64 {
        self.store.spend_this_week_usd().unwrap_or(0.0)
    }

    /// Per-model spend + token counts for the last 5 hours.
    pub fn spend_by_model_5h(&self) -> Vec<(String, f64, u64, u64)> {
        self.store.spend_by_model_5h().unwrap_or_default()
    }

    /// Per-model spend + token counts for today, for the `/usage` overlay.
    pub fn spend_by_model_today(&self) -> Vec<(String, f64, u64, u64)> {
        self.store.spend_by_model_today().unwrap_or_default()
    }

    /// Per-model spend + token counts for this ISO week.
    pub fn spend_by_model_week(&self) -> Vec<(String, f64, u64, u64)> {
        self.store.spend_by_model_week().unwrap_or_default()
    }

    /// Daily/monthly/weekly caps from config, for the `/usage` overlay gauges.
    pub fn budget_caps(&self) -> (Option<f64>, Option<f64>, Option<f64>) {
        (
            self.config.mesh.daily_budget_usd,
            self.config.mesh.monthly_cap_usd,
            self.config.mesh.weekly_budget_usd,
        )
    }

    /// Per-provider, per-window fraction from `subscription_usage` (for display fallback when
    /// the statusline cache is stale). Returns `HashMap<provider, HashMap<window_kind, fraction>>`.
    pub fn bridge_fractions(
        &self,
    ) -> std::collections::HashMap<String, std::collections::HashMap<String, f64>> {
        self.store.bridge_fractions().unwrap_or_default()
    }

    /// Seconds since the claude subscription quota was last updated (`None` if never). The CLI
    /// gates its on-demand rate-limit probe on this so it refreshes at most every few minutes.
    pub fn claude_quota_age_secs(&self) -> Option<i64> {
        self.store.subscription_age_secs("claude-cli")
    }

    /// Seed the subscription-usage store from an externally-observed window fraction (the
    /// Claude/Codex rate-limit caches the CLI reads). Forge otherwise only learns a subscription's
    /// usage when it runs a turn on that bridge — usage racked up *outside* Forge would read as 0%,
    /// making the mesh think the plan is fresh. `pct` is 0–100; `None` is skipped. The recorded row
    /// has no reset time, so it stays live until a real in-turn QuotaUpdate replaces it.
    ///
    /// Only for LIVE readings (observation time = now). Cache-derived readings (codex rollout
    /// files) must use [`Self::seed_subscription_quota_at`] with their true observation time, or a
    /// re-seeded hours-old reading would mask fresher data in the shared codex quota bucket.
    pub fn seed_subscription_quota(&self, provider: &str, window: &str, pct: Option<f64>) {
        self.seed_subscription_quota_at(provider, window, pct, None);
    }

    /// [`Self::seed_subscription_quota`] with the reading's true OBSERVATION time (epoch secs) —
    /// e.g. the codex rollout line's `timestamp` / file mtime. `Store::record_quota_at` drops the
    /// write entirely when the store already holds a newer observation for that window, so stale
    /// re-seeds can never regress a fresher reading. `observed_at: None` means "observed now".
    pub fn seed_subscription_quota_at(
        &self,
        provider: &str,
        window: &str,
        pct: Option<f64>,
        observed_at: Option<i64>,
    ) {
        let Some(pct) = pct else { return };
        let frac = (pct / 100.0).clamp(0.0, 1.0);
        let status = if frac >= 0.98 {
            forge_types::QuotaStatus::Exhausted
        } else if frac >= 0.80 {
            forge_types::QuotaStatus::Warning
        } else {
            forge_types::QuotaStatus::Ok
        };
        let hint = forge_types::QuotaHint {
            provider: provider.to_string(),
            window: window.to_string(),
            status,
            resets_at: None,
            fraction_used: Some(frac),
        };
        let _ = match observed_at {
            Some(ts) => self.store.record_quota_at(&hint, ts),
            None => self.store.record_quota(&hint),
        };
    }

    /// After a fresh [`forge_types::QuotaHint`] is recorded, look back at that window's history
    /// and — if there's enough of it — derive a [`forge_types::QuotaPace`] projection and push it
    /// to the presenter for the statusline meter (mesh-routing.md). A no-op (no event) when
    /// the hint carries no fraction, or when there isn't yet enough history to project from
    /// (single sample, or samples too close together — see `compute_quota_pace`'s guard).
    pub(crate) fn emit_quota_pace(&mut self, hint: &forge_types::QuotaHint) {
        let Some(_fraction) = hint.fraction_used else {
            return;
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let since = now - forge_types::QUOTA_PACE_LOOKBACK_SECS;
        let Ok(history) = self
            .store
            .quota_history_since(&hint.provider, &hint.window, since)
        else {
            return;
        };
        let Some(pace) = forge_types::compute_quota_pace(&history, hint.resets_at, now) else {
            return;
        };
        self.presenter.emit(forge_types::PresenterEvent::QuotaPace {
            provider: hint.provider.clone(),
            window: hint.window.clone(),
            rate_per_hour: pace.rate_per_hour,
            projected_fraction_at_reset: pace.projected_fraction_at_reset,
            exhaustion_warning: pace.exhaustion_warning,
        });
    }

    /// Advance the temper through the SHIFT+TAB cycle, persist it, and return the new temper
    /// (RFC/temper-modes). Takes effect on the next turn's permission decisions.
    pub fn cycle_temper(&mut self) -> PermissionMode {
        self.set_temper(self.mode.cycle_next())
    }

    /// Set the temper to a specific mode (the `/mode` picker), persist it, and return it. Unlike
    /// the cycle this can reach `Bypass`/Full, since the picker is an explicit, deliberate choice.
    pub fn set_temper(&mut self, mode: PermissionMode) -> PermissionMode {
        if mode == PermissionMode::Plan && self.mode != PermissionMode::Plan {
            self.pre_plan_mode = Some(self.mode);
        } else if mode != PermissionMode::Plan {
            self.pre_plan_mode = None;
        }
        self.mode = mode;
        self.config.permission_mode = mode;
        let _ = self
            .store
            .update_session_mode(&self.id, &format!("{:?}", self.mode));
        self.mode
    }
}
