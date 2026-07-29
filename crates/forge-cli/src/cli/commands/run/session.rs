//! Session construction and startup integrations for CLI surfaces.

use super::*;

fn should_refresh_codex_quota(mock: bool, pin: Option<&str>) -> bool {
    if mock {
        return false;
    }
    match pin.map(forge_config::provider_of) {
        // Unpinned/bare ids may route through Codex, so preserve the routing-pressure refresh.
        None | Some("") => true,
        // Any fully-qualified pin bypasses mesh selection. A Codex response carries fresh quota
        // headers itself, so a separate pre-turn Luna probe cannot affect the decision and only
        // adds subscription burn plus startup latency.
        Some(_) => false,
    }
}

fn remove_recursive_self_mcp(
    mcp_config: &mut forge_config::McpConfig,
    self_exe_name: Option<&str>,
) {
    mcp_config.servers.retain(|server| {
        let forge_config::McpTransport::Stdio { command, args, .. } = &server.transport else {
            return true;
        };
        let is_self_binary = self_exe_name.is_some_and(|name| {
            std::path::Path::new(command)
                .file_name()
                .is_some_and(|file| file.to_string_lossy() == name)
        });
        let is_mcp_agent_invocation =
            args.iter().any(|arg| arg == "mcp") && args.iter().any(|arg| arg == "agent");
        !(is_self_binary && is_mcp_agent_invocation)
    });
}

pub(crate) async fn build_session_with(
    presenter: Box<dyn Presenter>,
    mock: bool,
    mode: Option<Mode>,
    resume: Option<String>,
    pin: Option<String>,
    suppress_mcp_announce: bool,
) -> Result<Session> {
    build_session_with_self_mcp(
        presenter,
        mock,
        mode,
        resume,
        pin,
        suppress_mcp_announce,
        true,
        None,
    )
    .await
}

/// Build a session, optionally suppressing the self-MCP injection (see the `disable_self_mcp`
/// doc below). `build_session_with` is the normal entrypoint (self-MCP allowed); `forge mcp
/// agent` calls this directly with `disable_self_mcp = false` to break the recursion.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn build_session_with_self_mcp(
    presenter: Box<dyn Presenter>,
    mock: bool,
    mode: Option<Mode>,
    resume: Option<String>,
    pin: Option<String>,
    suppress_mcp_announce: bool,
    allow_self_mcp: bool,
    session_cwd: Option<&str>,
) -> Result<Session> {
    if let Some(session_id) = resume.as_deref() {
        if crate::open_store()?
            .session_handoff_blocked(session_id)
            .context("checking Anywhere handoff state")?
        {
            anyhow::bail!(
                "session {session_id} is frozen by an Anywhere handoff and cannot be resumed"
            );
        }
    }
    // Make any keyring-stored provider keys visible to the provider client.
    forge_config::inject_provider_keys();
    // …and the search-API key visible to the web_search tool.
    forge_config::inject_search_keys();

    let mut config = forge_config::load().context("loading configuration")?;
    if let Some(m) = mode {
        config.permission_mode = m.into();
    }
    // Capture the MCP config before `config` is moved into the Session; connect after the session
    // is built so its presenter can show the connection status.
    let mut mcp_config = config.mcp.clone();
    // Self-MCP: inject a sub-Forge MCP agent server so forge_chat / forge_assay are available
    // as native tools. Skipped if already declared (prevents duplicate "forge" prefix), and
    // skipped entirely when `allow_self_mcp` is false — `forge mcp agent` builds its OWN session
    // through this same function, and without this guard each spawned agent injected another
    // "forge" MCP server pointing at `mcp agent`, which something then eagerly connected
    // (spawned) immediately: a real, observed runaway self-fork chain (one child every
    // ~200-300ms, no depth limit, OOM'd the machine in minutes). `forge mcp agent` IS the
    // self-MCP tool surface already — it must never try to spawn another copy of itself.
    if !allow_self_mcp {
        // Not enough to just skip the dynamic injection below: `forge import claude` (or a
        // user's own `.forge/mcp.toml`) can ALSO persist an explicit "forge" server entry
        // (copied verbatim from a `.mcp.json` like the one this binary documents in its own
        // `--help`). `forge mcp agent` loads the exact same `mcp_config` as every other
        // session, so a persisted entry bypasses the injection guard entirely and still gets
        // eagerly connected (= spawned) by `connect_active()` — this is what actually kept
        // reproducing the fork bomb after the injection-only fix shipped. Strip any stdio
        // server that resolves to THIS SAME BINARY invoked with `mcp agent`, regardless of
        // what it's named in the config (covers a renamed entry too, not just literally
        // "forge").
        let self_exe_name = std::env::current_exe().ok().and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        });
        remove_recursive_self_mcp(&mut mcp_config, self_exe_name.as_deref());
    } else if config.self_mcp && !mcp_config.servers.iter().any(|s| s.name == "forge") {
        let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("forge"));
        mcp_config.servers.insert(
            0,
            forge_config::McpServerConfig {
                name: "forge".to_string(),
                transport: forge_config::McpTransport::Stdio {
                    command: exe.to_string_lossy().into_owned(),
                    args: vec!["mcp".to_string(), "agent".to_string()],
                    env: std::collections::HashMap::new(),
                },
                auth: None,
                secret_env: vec![],
                enabled: true,
            },
        );
    }
    let config_has_mcp = mcp_config.active_servers().next().is_some();
    let lattice_enabled = config.lattice.enabled;
    let config_lattice_watch = config.lattice.watch;
    let config_default_effort = config.mesh.default_effort.clone();

    // Normalize before any provider-specific startup work so a pinned session only initializes
    // the provider it can actually use.
    let pin = pin.map(|p| forge_provider::normalize_model_id(&p).into_owned());

    let store = Arc::new(open_store()?);
    // Never construct a session router from an expired Codex pressure reading. The helper uses a
    // fresh CLI rollout when available, otherwise performs one bounded minimal OAuth probe; it
    // updates the shared codex-oauth/codex-cli quota bucket before this session's first route.
    // A pinned non-Codex model never routes to Codex, so probing its keyring/quota is irrelevant
    // startup work and can block the requested provider behind an unavailable Secret Service.
    if should_refresh_codex_quota(mock, pin.as_deref()) {
        crate::cli::commands::models::refresh_codex_quota(&store).await;
    }
    let store_for_lattice = Arc::clone(&store);
    // Startup hint: if models are benched from a prior run/probe, tell the user how to recheck
    // (docs/features/mesh-routing.md — we never auto-probe, so a stale bench is the user's to clear).
    let mut presenter = presenter;
    if let Ok(report) = store.current_benched_report() {
        if !report.is_empty() {
            presenter.emit(forge_tui::PresenterEvent::Warning(format!(
                "{} model(s) benched (rate-limited/unavailable) — `forge models --probe` to recheck",
                report.len()
            )));
        }
    }

    // Auto-discovery: build a live model catalog so the mesh routes to the best usable model
    // (docs/features/mesh-routing.md). Skipped for the offline mock and when disabled.
    //
    // Cache-first: if a catalog from the last 24 h exists on disk, use it instantly and kick off
    // a background refresh so the NEXT startup is also fast. On first run (or stale cache) we
    // do the full network discovery (bounded at 15 s) and save it for next time.
    let catalog = if !mock && config.mesh.auto_discover {
        if let Some(cached) = load_cached_catalog() {
            // Fast path — instant startup. Refresh in background for the next run.
            let cfg = config.clone();
            tokio::spawn(async move {
                let fresh = discover_catalog(&cfg).await;
                save_catalog(&fresh);
            });
            Some(cached)
        } else {
            // First run or stale cache — block on discovery, then persist the result.
            const DISCOVERY_BUDGET: std::time::Duration = std::time::Duration::from_secs(15);
            match tokio::time::timeout(DISCOVERY_BUDGET, discover_catalog(&config)).await {
                Ok(cat) => {
                    save_catalog(&cat);
                    Some(cat)
                }
                Err(_) => {
                    presenter.emit(forge_tui::PresenterEvent::Warning(format!(
                        "model auto-discovery exceeded {}s — using built-in defaults for now; run \
                         `forge models` to refresh once your network/providers respond",
                        DISCOVERY_BUDGET.as_secs()
                    )));
                    None
                }
            }
        }
    } else {
        None
    };

    // Validate the pinned model so unknown ids fail fast with a clear message rather than a
    // confusing provider "Resolver error" at the first API call.
    if let Some(id) = pin.as_deref() {
        let prefix = forge_config::provider_of(id);
        // A prefixed id whose provider isn't a recognized one is clearly invalid — hard stop, even
        // when discovery is off/timed-out and there's no catalog to check against (it would
        // otherwise pass straight through to a raw resolver error every turn).
        if !prefix.is_empty() && !is_known_provider_prefix(prefix) {
            anyhow::bail!(
                "unknown model '{id}': '{prefix}' is not a known provider. \
                 Run `forge models` to see usable ids, or `forge auth` to add a provider."
            );
        }
        // With a catalog, also flag a known-provider id that isn't in it (likely a typo). This
        // stays a soft warning: a brand-new model may simply not be discovered yet.
        if let Some(cat) = catalog.as_ref() {
            if !cat.models().contains(&id.to_string()) {
                let suggestions: Vec<&str> = cat
                    .models()
                    .iter()
                    .filter(|m| m.starts_with(prefix))
                    .map(String::as_str)
                    .take(5)
                    .collect();
                let hint = if suggestions.is_empty() {
                    format!("no '{prefix}' models in catalog — run `forge models` to see what's available")
                } else {
                    format!("try: {}", suggestions.join(", "))
                };
                presenter.emit(forge_tui::PresenterEvent::Warning(format!(
                    "unknown model '{id}' — {hint}"
                )));
            }
        }
    }

    let catalog = catalog
        .map(|catalog| crate::cli::commands::models::apply_outcome_calibration(catalog, &store));
    let ctx_windows = crate::open_store()
        .ok()
        .and_then(|s| s.all_model_contexts().ok())
        .unwrap_or_default();
    let workspace_root = match session_cwd {
        Some(cwd) => std::path::PathBuf::from(cwd)
            .canonicalize()
            .with_context(|| format!("resolving session workspace {cwd}"))?,
        None if resume.is_some() => {
            let prefix = resume.as_deref().expect("resume checked above");
            let full = resolve_session(&store, prefix)?;
            let cwd = store
                .session_cwd(&full)?
                .ok_or_else(|| anyhow::anyhow!("resumed session has no workspace"))?;
            std::path::PathBuf::from(cwd)
                .canonicalize()
                .context("resumed session workspace is unavailable")?
        }
        None => std::env::current_dir()?,
    };
    let workspace_cwd = workspace_root.display().to_string();
    let repo_key = workspace_cwd.clone();
    let repo_boosts = crate::open_store()
        .ok()
        .and_then(|s| s.duel_boosts(&repo_key).ok())
        .unwrap_or_default();
    let (provider, router) = build_provider_and_router(
        &config,
        mock,
        pin,
        catalog.clone(),
        ctx_windows,
        repo_boosts,
    );

    // Build the code-intelligence index up front so it can be shared between the model-facing
    // `lattice` tool and the turn's auto-injection (code-intelligence.md). Cheap to construct; it
    // reads whatever `forge lattice update` last persisted.
    let lattice = (!mock && lattice_enabled).then(|| {
        let root = workspace_root.clone();
        Arc::new(forge_index::Lattice::new(store_for_lattice, &root))
    });
    let mut tools = ToolRegistry::with_core_tools_in(&workspace_root);
    // Opt-in OS sandbox and/or scoped build-target dir: replace the default shell tool with one
    // that confines filesystem writes to the workspace via Landlock (Linux; no-op elsewhere) and/or
    // relocates cargo's CARGO_TARGET_DIR outside the (possibly read-only) workspace so a
    // bypass-mode agent can compile-check its own edits under confinement. Shared with the
    // `mcp-serve` bridge path via `sandboxed_shell_tool` so the two can't drift.
    if let Some(shell_tool) = sandboxed_shell_tool_in(&config, &workspace_root) {
        tools.register(Box::new(shell_tool));
    }
    if let Some(lat) = &lattice {
        tools.register(Box::new(forge_tools::LatticeTool::new(Arc::clone(lat))));
        // Auto-index (and auto-embed when enabled) in the background so the graph is fresh without
        // a manual `forge lattice update` — "automatic under the hood". Incremental + non-blocking;
        // the watcher keeps it fresh thereafter. Errors are swallowed (best-effort, additive).
        let lat_bg = Arc::clone(lat);
        let embeddings = config.lattice.embeddings.clone();
        tokio::spawn(async move {
            // `Lattice::update()` is fully synchronous and CPU-bound (walks the repo, tree-sitter
            // parses every file, writes SQLite). Running it inside a plain async task occupies a
            // tokio *worker* thread for its whole duration — on a low-core machine (runtime sized
            // to `num_cpus`) that starves the executor and the first turn's `route_hinted` never
            // gets scheduled, so `forge run` hangs right after `● session`. Offload to the blocking
            // pool so worker threads stay free. (`spawn_blocking` JoinError on panic → treat as
            // "not updated" rather than propagating.)
            let lat_update = Arc::clone(&lat_bg);
            let updated = tokio::task::spawn_blocking(move || lat_update.update().is_ok())
                .await
                .unwrap_or(false);
            if updated {
                if let Some((embedder, _)) = forge_provider::select_embedder(&embeddings) {
                    let _ = lat_bg.embed_pending(embedder.as_ref(), 64).await;
                }
            }
        });
    }

    let lsp_config = config.lsp.clone();
    let mut session = match resume {
        Some(ref prefix) => {
            let full = resolve_session(&store, prefix)?;
            Session::resume(store, provider, router, tools, presenter, config, &full)
                .with_context(|| format!("resuming session {full}"))?
        }
        None => Session::start(
            store,
            provider,
            router,
            tools,
            presenter,
            config,
            &workspace_cwd,
        )
        .context("starting session")?,
    };
    session.set_catalog(catalog);
    // Seed the effort pin from config if set (`mesh.default_effort`).
    if let Some(ref s) = config_default_effort {
        if let Some(e) = forge_types::EffortLevel::parse(s) {
            session.set_effort(Some(e));
        }
    }
    // Share the index with the session so turns auto-inject relevant code and agent edits reindex
    // in-turn (code-intelligence.md). Empty index → nothing injected (additive guarantee).
    // Also start the background watcher so external editor edits reindex automatically.
    if let Some(lat) = &lattice {
        if config_lattice_watch {
            let cwd = workspace_root.clone();
            // Scope the recursive watch to the nearest PROJECT ROOT, and refuse to watch all of
            // $HOME (pathological: pulls in .cargo / cloned .git trees / caches → thousands of
            // inotify watches + a slow initial walk). `None` ⇒ no sensible root → skip the watcher.
            let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
            match forge_index::resolve_watch_root(&cwd, home.as_deref()) {
                None => session.notify_error(
                    "watch & reindex skipped: launched in the home directory with no project root \
                     — open a project folder (one with a .git) to enable auto-reindex",
                ),
                Some(root) => {
                    // Build the watcher on a detached thread and DELIVER it to the session through a
                    // channel, so NOTHING about watcher setup gates TUI startup — not a recursive
                    // inotify registration (which blocks uninterruptibly on WSL2's 9p DrvFs and used
                    // to hang `forge chat`), nor the polling backend's synchronous initial tree scan
                    // (slow over a remote/9p link). On a non-native fs spawn_watcher transparently
                    // uses polling so auto-reindex still works there. The session holds the receiver,
                    // so the watcher is owned per-session and dropped when the session ends (no leak
                    // across repeated build_session calls — bench/replay); the thread exits after the
                    // send. A setup error is non-fatal and intentionally silent (no caveat).
                    let lat2 = Arc::clone(lat);
                    let (tx, rx) = std::sync::mpsc::channel();
                    std::thread::spawn(move || {
                        if let Ok(watcher) = forge_index::spawn_watcher(
                            lat2,
                            &root,
                            std::time::Duration::from_millis(400),
                        ) {
                            let _ = tx.send(watcher);
                        }
                    });
                    session.set_lattice_watcher(Some(rx));
                }
            }
        }
    }
    session.set_lattice(lattice);

    // Attach the command/skill catalog so the model can discover + load Forge's own skills via
    // the `use_skill` tool (instead of hunting ~/.claude). Cheap, sync, pure.
    let skill_catalog = forge_skills::Catalog::load(&forge_config::command_sources());
    session.set_skills(Some(std::sync::Arc::new(skill_catalog)));

    // Connect external MCP servers (mcp-client.md). Skipped for the offline mock. Per-server
    // failures are isolated inside connect_all (each lands `failed` with a reason); we surface the
    // whole listing once on a fresh session (resume suppresses it — the transcript separator
    // already orients the user, and the MCP panel is always reachable via `/mcp`).
    if !mock && config_has_mcp {
        // Connect MCP servers in the BACKGROUND so a slow/unreachable server can't delay TUI startup
        // by up to connect_timeout (20s default per server) — the same non-blocking pattern
        // `mcp-serve` uses. `connecting()` marks every active server `Reconnecting` and advertises
        // the MCP meta-tools immediately (so `is_empty()` is false and the tool surface is ready),
        // then a detached task connects them; each flips to connected/failed in the `/mcp` panel as
        // it resolves, and the first `mcp_call` lazily waits on its own server. No startup op should
        // gate the UI (cf. the 9p watcher hang).
        let manager = std::sync::Arc::new(forge_mcp::McpManager::connecting(&mcp_config));
        let bg = std::sync::Arc::clone(&manager);
        tokio::spawn(async move { bg.connect_active().await });
        session.set_mcp(Some(manager));
        if resume.is_none() && !suppress_mcp_announce {
            session.announce_mcp();
        }
    }
    if lsp_config.enabled {
        session.set_lsp(Some(std::sync::Arc::new(
            forge_lsp::LspRegistry::from_config(&lsp_config),
        )));
    }
    Ok(session)
}

/// Build a session with the default surface (TUI on a tty, else plain).
pub(crate) async fn build_session(
    mock: bool,
    mode: Option<Mode>,
    tui: bool,
    resume: Option<String>,
    pin: Option<String>,
) -> Result<Session> {
    let presenter: Box<dyn Presenter> = if tui && std::io::stdout().is_terminal() {
        Box::new(TuiPresenter::new().context("initializing TUI")?)
    } else {
        if tui {
            eprintln!("forge: --tui needs an interactive terminal; falling back to plain output");
        }
        Box::new(HeadlessPresenter::default())
    };
    build_session_with(presenter, mock, mode, resume, pin, false).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stdio_server(name: &str, command: &str, args: &[&str]) -> forge_config::McpServerConfig {
        forge_config::McpServerConfig {
            name: name.to_string(),
            transport: forge_config::McpTransport::Stdio {
                command: command.to_string(),
                args: args.iter().map(|arg| (*arg).to_string()).collect(),
                env: Default::default(),
            },
            auth: None,
            secret_env: vec![],
            enabled: true,
        }
    }

    #[test]
    fn mcp_agent_removes_only_recursive_self_servers() {
        let mut config = forge_config::McpConfig {
            servers: vec![
                stdio_server("renamed-self", "/opt/bin/forge", &["mcp", "agent"]),
                stdio_server("other-forge-command", "/opt/bin/forge", &["mcp", "serve"]),
                stdio_server("other-agent", "/opt/bin/helper", &["mcp", "agent"]),
            ],
            ..Default::default()
        };

        remove_recursive_self_mcp(&mut config, Some("forge"));

        let names: Vec<_> = config
            .servers
            .iter()
            .map(|server| server.name.as_str())
            .collect();
        assert_eq!(names, ["other-forge-command", "other-agent"]);
    }

    #[test]
    fn mcp_agent_keeps_config_when_executable_identity_is_unavailable() {
        let mut config = forge_config::McpConfig {
            servers: vec![stdio_server("forge", "forge", &["mcp", "agent"])],
            ..Default::default()
        };

        remove_recursive_self_mcp(&mut config, None);

        assert_eq!(config.servers.len(), 1);
    }

    #[test]
    fn codex_quota_refresh_only_runs_when_the_session_can_use_codex() {
        assert!(should_refresh_codex_quota(false, None));
        assert!(should_refresh_codex_quota(false, Some("bare-model")));
        assert!(!should_refresh_codex_quota(
            false,
            Some("codex-cli::gpt-5.4-mini")
        ));
        assert!(!should_refresh_codex_quota(
            false,
            Some("codex-oauth::gpt-5.6-luna")
        ));
        assert!(!should_refresh_codex_quota(
            false,
            Some("claude-cli::sonnet")
        ));
        assert!(!should_refresh_codex_quota(false, Some("openai::gpt-5.4")));
        assert!(!should_refresh_codex_quota(true, None));
    }
}
