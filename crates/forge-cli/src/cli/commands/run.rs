use anyhow::{Context, Result};
use std::io::IsTerminal;
use std::sync::Arc;

use forge_core::Session;
use forge_tools::ToolRegistry;
use forge_tui::{HeadlessPresenter, Presenter, TuiPresenter};

use crate::*;

/// Build the `shell` tool's Landlock sandbox and/or scoped `CARGO_TARGET_DIR` carve-out from
/// `[shell]` config (`sandbox` / `scoped_cargo_target`, ADR-0008 + PR #521). Returns `None` when
/// both knobs are off, in which case the caller should keep the plain `ShellTool::default()`
/// already registered by `ToolRegistry::with_core_tools()`. Shared by `forge run` (this file) and
/// the `mcp-serve` CLI-bridge path (`crate::mcp_serve::run`) so the two entry points can't drift —
/// a bridged claude/codex agent gets the same compile-check carve-out as a direct `forge run`
/// session.
pub(crate) fn sandboxed_shell_tool(
    config: &forge_config::Config,
) -> Option<forge_tools::ShellTool> {
    if !(config.shell.sandbox || config.shell.scoped_cargo_target) {
        return None;
    }
    let writable = config
        .shell
        .sandbox_writable
        .iter()
        .map(std::path::PathBuf::from)
        .collect();
    let cargo_target_base = config.shell.scoped_cargo_target.then(|| {
        config
            .shell
            .scoped_cargo_target_dir
            .clone()
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("forge-cargo-target"))
    });
    Some(forge_tools::ShellTool {
        policy: forge_tools::SandboxPolicy {
            enabled: config.shell.sandbox,
            writable,
            cargo_target_base,
        },
    })
}

/// A finished `/duel` result: the comparable report plus the still-alive worktree guards for
/// every candidate (the picker owns picking a winner — merging it back and dropping every guard).
/// Named so the background task ([`spawn_duel`]) and the done-signal drain that consumes it don't
/// need to spell out the nested `Arc<Mutex<Option<(...)>>>` type.
pub(crate) type PendingDuel = Option<(
    forge_core::duel::DuelReport,
    Vec<forge_core::worktree::WorktreeGuard>,
)>;

mod atfiles;
pub(crate) use atfiles::*;
mod copy;
pub(crate) use copy::*;
mod pickers;
pub(crate) use pickers::*;
mod dispatch;
pub(crate) use dispatch::*;
mod driver;
pub(crate) use driver::*;

/// Keep the command palette in sync with the `/command` token at the cursor (input end): open +
/// filter when one is present anywhere on the line, close when not (`//` escape yields no token).
/// Fill in missing bridge-provider percentages on the usage overlay from the store's
/// `subscription_usage` table (set via rate_limit_event during Forge turns). Used as a
/// fallback when the statusline cache file is stale or missing.
/// Populate the overlay's subscription utilisation %s, preferring the STORE's fractions (seeded
/// from the rate-limit caches at startup AND refreshed live on every CLI-bridge turn via
/// rate_limit_event) over the raw caches. This is the real staleness fix: a fresh Forge claude/
/// codex turn updates the store, so the overlay reflects it instead of the frozen statusline cache.
/// The "Xh ago" note is shown only when the claude reading is still the seeded cache value (i.e. no
/// live turn refreshed it this session) — when a turn has, the value is current and unmarked.
pub(crate) fn fill_subscription_pcts(
    overlay: &mut forge_tui::UsageOverlay,
    fracs: &std::collections::HashMap<String, std::collections::HashMap<String, f64>>,
    bstats: &bridge_stats::BridgeStats,
) {
    let store = |p: &str, w: &str| fracs.get(p).and_then(|m| m.get(w)).copied();
    // Cache as the base; override with the store only when it carries a genuinely DIFFERENT (live,
    // turn-recorded) value, so we never show a store reading staler than the cache. Returns the %
    // and whether it came from a live override.
    let pick = |cache: Option<f64>, st: Option<f64>| -> (Option<f64>, bool) {
        match (st, cache) {
            (Some(s), Some(c)) => {
                let sp = s * 100.0;
                if (sp - c).abs() > 1e-6 {
                    (Some(sp), true)
                } else {
                    (Some(c), false)
                }
            }
            (Some(s), None) => (Some(s * 100.0), true),
            (None, c) => (c, false),
        }
    };
    let (c5, _) = pick(bstats.claude_5h_pct, store("claude-cli", "five_hour"));
    let (cw, cw_live) = pick(bstats.claude_weekly_pct, store("claude-cli", "weekly"));
    overlay.claude_5h_pct = c5;
    overlay.claude_weekly_pct = cw;
    let (x5, _) = pick(bstats.codex_5h_pct, store("codex-cli", "five_hour"));
    let (xw, _) = pick(bstats.codex_weekly_pct, store("codex-cli", "weekly"));
    overlay.codex_5h_pct = x5;
    overlay.codex_weekly_pct = xw;
    // A live turn refreshed the weekly reading → it's current; otherwise surface the cache age.
    overlay.claude_rl_age_secs = if cw_live {
        None
    } else {
        bstats.claude_rl_age_secs
    };
}

pub(crate) fn sync_palette_to_slash_token(app: &mut forge_tui::App) {
    let cur = app.input_cursor.min(app.input.len());
    // Cursor-anchored: drive the palette only from a `/command` token the cursor sits *within*.
    // `slash_token_at` otherwise falls back to the last token on the line, which kept the palette
    // open after a trailing space (so it never closed once you started typing args). Requiring the
    // cursor to be inside the token closes it the moment the cursor moves past the command name.
    let tok = forge_tui::slash_token_at(&app.input, cur).filter(|t| cur >= t.start && cur <= t.end);
    match tok {
        Some(tok) if app.palette.open => {
            app.palette.query = tok.name;
            app.palette.clamp();
        }
        Some(tok) => app.palette.open_with(&tok.name),
        None => app.palette.close(),
    }
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
        let self_exe_name = std::env::current_exe()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
        mcp_config.servers.retain(|s| {
            let forge_config::McpTransport::Stdio { command, args, .. } = &s.transport else {
                return true;
            };
            let is_self_binary = self_exe_name.as_deref().is_some_and(|n| {
                std::path::Path::new(command)
                    .file_name()
                    .map(|f| f.to_string_lossy() == n)
                    .unwrap_or(false)
            });
            let is_mcp_agent_invocation =
                args.iter().any(|a| a == "mcp") && args.iter().any(|a| a == "agent");
            !(is_self_binary && is_mcp_agent_invocation)
        });
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

    let store = Arc::new(open_store()?);
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

    // Normalize legacy underscore-prefix aliases (codex_cli:: → codex-cli::) so that
    // `--model codex_cli::gpt-5.4-mini` works identically to the canonical hyphen form.
    let pin = pin.map(|p| forge_provider::normalize_model_id(&p).into_owned());

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

    let ctx_windows = crate::open_store()
        .ok()
        .and_then(|s| s.all_model_contexts().ok())
        .unwrap_or_default();
    // Per-repo routing-learning boosts from past `/duel` outcomes (docs/features/duel.md) — same
    // repo-key convention as `Session::run_duel`'s recording side (the cwd's display string).
    let repo_key = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
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
        let root = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        Arc::new(forge_index::Lattice::new(store_for_lattice, &root))
    });
    let mut tools = ToolRegistry::with_core_tools();
    // Opt-in OS sandbox and/or scoped build-target dir: replace the default shell tool with one
    // that confines filesystem writes to the workspace via Landlock (Linux; no-op elsewhere) and/or
    // relocates cargo's CARGO_TARGET_DIR outside the (possibly read-only) workspace so a
    // bypass-mode agent can compile-check its own edits under confinement. Shared with the
    // `mcp-serve` bridge path via `sandboxed_shell_tool` so the two can't drift.
    if let Some(shell_tool) = sandboxed_shell_tool(&config) {
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
        None => {
            // `forge serve` drives sessions in per-session directories (worktrees) — the
            // process cwd is only the default.
            let cwd = match session_cwd {
                Some(c) => c.to_string(),
                None => std::env::current_dir()?.display().to_string(),
            };
            Session::start(store, provider, router, tools, presenter, config, &cwd)
                .context("starting session")?
        }
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
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
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

pub(crate) async fn run(
    prompt: String,
    mock: bool,
    mode: Option<Mode>,
    tui: bool,
    resume: Option<String>,
    pin: Option<String>,
    output_format: OutputFormat,
) -> Result<()> {
    if prompt.trim().is_empty() {
        anyhow::bail!("empty prompt — usage: forge run \"<your task>\"");
    }
    // A first-time user's `forge run "hi"` would otherwise dead-end with no provider; offer the
    // guided wizard (no-ops on non-tty / once configured), same as `chat()`.
    maybe_first_run_setup(mock)?;

    // One-shot slash support: `forge run "/rust <task>"` expands a catalog command/skill exactly
    // like the interactive dispatcher — without this the literal `/rust` reaches the model as
    // prose and it guesses at the intent. Unknown tokens (absolute paths, TUI-only builtins)
    // pass through verbatim; `//…` escapes a literal leading slash, mirroring chat.
    let (prompt, guidance, tier) = expand_one_shot_slash(&prompt)?;

    // stream-json: emit NDJSON events on stdout via the StreamJsonPresenter (no TUI, no heartbeat —
    // stdout stays a clean machine-readable event stream). Ctrl-C still returns partial output.
    if output_format == OutputFormat::StreamJson {
        let presenter: Box<dyn Presenter> = Box::new(forge_tui::StreamJsonPresenter::new());
        let mut session = build_session_with(presenter, mock, mode, resume, pin, true).await?;
        let turn = session.run_turn_with(&prompt, &guidance, tier);
        tokio::pin!(turn);
        let result = tokio::select! {
            r = &mut turn => r.map(|_| ()).context("running agent turn"),
            _ = tokio::signal::ctrl_c() => Ok(()),
        };
        result?;
        return Ok(());
    }

    let mut session = build_session(mock, mode, tui, resume, pin).await?;

    // TUI mode handles its own Ctrl-C (crossterm) + spinner; keep it unchanged.
    if tui {
        session
            .run_turn_with(&prompt, &guidance, tier)
            .await
            .context("running agent turn")?;
        // Hold the final frame until the user quits (Esc / Ctrl-C).
        let _ = session.read_line();
        return Ok(());
    }

    // Headless heartbeat: a long model call streams nothing until the first token, so tick
    // "working… Ns" to stderr to show the turn is alive. Skipped for `--mock` (instant).
    let heartbeat = (!mock).then(|| {
        tokio::spawn(async {
            let start = std::time::Instant::now();
            let mut iv = tokio::time::interval(std::time::Duration::from_secs(2));
            iv.tick().await; // immediate first tick — skip it
            loop {
                iv.tick().await;
                eprint!("\r\x1b[2m⧖ working… {}s\x1b[0m", start.elapsed().as_secs());
                let _ = std::io::Write::flush(&mut std::io::stderr());
            }
        })
    });

    // Race the turn against Ctrl-C so a hard kill doesn't discard partial output: on interrupt we
    // drop the turn future (it stops at its next await) and return what already streamed.
    let result = {
        let turn = session.run_turn_with(&prompt, &guidance, tier);
        tokio::pin!(turn);
        tokio::select! {
            r = &mut turn => r.map(|_| ()).context("running agent turn"),
            _ = tokio::signal::ctrl_c() => {
                eprintln!("\r\x1b[K\x1b[2m⧖ interrupted — stopping turn (partial output kept)\x1b[0m");
                Ok(())
            }
        }
    };
    if let Some(h) = heartbeat {
        h.abort();
        eprint!("\r\x1b[K"); // clear the heartbeat line
        let _ = std::io::Write::flush(&mut std::io::stderr());
    }
    result?;
    Ok(())
}

/// Resolve a one-shot `/command` or skill prompt against the file catalog (the same
/// `Catalog::resolve` the interactive dispatcher uses) so `forge run "/rust <task>"` behaves like
/// typing it in the TUI. Returns `(prompt, guidance, tier)`:
/// - `//foo` escapes to a literal `/foo` prompt (mirrors chat).
/// - A command expands to its prompt (+ guidance/tier); a skill with a task runs the task under
///   the skill's methodology.
/// - A bare skill or missing args is a usage error — one-shot has no "next turn" to prime.
/// - Project-scope definitions require `[commands] trust_project = true`: the interactive
///   run-again-to-confirm gate has no headless equivalent, so default-deny with a pointer.
/// - Anything unresolved (absolute paths, unknown tokens, TUI-only builtins) passes through
///   verbatim.
fn expand_one_shot_slash(
    raw: &str,
) -> Result<(String, Vec<String>, Option<forge_types::TaskTier>)> {
    let t = raw.trim();
    if let Some(rest) = t.strip_prefix("//") {
        return Ok((format!("/{rest}"), Vec::new(), None));
    }
    if !t.starts_with('/') {
        return Ok((t.to_string(), Vec::new(), None));
    }
    let catalog = forge_skills::Catalog::load(&forge_config::command_sources());
    let trust_project = forge_config::load()
        .map(|c| c.commands.trust_project)
        .unwrap_or(false);
    use forge_skills::Resolved;
    match catalog.resolve(t) {
        Resolved::Command {
            cmd,
            prompt,
            guidance,
        } => {
            if cmd.scope == forge_skills::Scope::Project && !trust_project {
                anyhow::bail!(
                    "/{} is a project-scope command — set `[commands] trust_project = true` in \
                     your config, or run it interactively via `forge chat`",
                    cmd.name
                );
            }
            eprintln!("⚒ command · /{} ({})", cmd.name, cmd.scope.label());
            Ok((prompt, guidance, cmd.tier))
        }
        Resolved::Skill { meta, prompt } => {
            if meta.scope == forge_skills::Scope::Project && !trust_project {
                anyhow::bail!(
                    "/{} is a project-scope skill — set `[commands] trust_project = true` in \
                     your config, or run it interactively via `forge chat`",
                    meta.name
                );
            }
            if prompt.trim().is_empty() {
                anyhow::bail!(
                    "skill /{name} needs a task in one-shot mode — usage: forge run \
                     \"/{name} <task>\"",
                    name = meta.name
                );
            }
            let skill = forge_skills::Skill::load(&meta);
            for w in &skill.warnings {
                eprintln!("⚠ {w}");
            }
            eprintln!("⚒ skill · {} ({})", meta.name, meta.scope.label());
            Ok((prompt, vec![skill.guidance()], meta.tier))
        }
        Resolved::MissingArgs { name, missing } => {
            let need = missing
                .iter()
                .map(|m| format!("<{m}>"))
                .collect::<Vec<_>>()
                .join(" ");
            anyhow::bail!("/{name} requires {need}")
        }
        Resolved::Unknown(_) => Ok((t.to_string(), Vec::new(), None)),
        // Unreachable here (the early returns above cover non-slash + `//` escapes), but the
        // catalog's own contract for it is "pass straight to run_turn" — honor that.
        Resolved::Plain(p) => Ok((p, Vec::new(), None)),
    }
}

pub(crate) async fn nl_cmd(query: String, mode: Option<Mode>) -> Result<()> {
    if query.trim().is_empty() {
        anyhow::bail!(
            "empty query — usage: forge nl \"what changed performance-wise since last week\""
        );
    }
    maybe_first_run_setup(false)?;
    // Gather shell context so the model can run the right commands.
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| ".".to_string());
    let git_ctx = {
        let branch = std::process::Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string());
        let log = std::process::Command::new("git")
            .args(["log", "--oneline", "-8"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string());
        match (branch, log) {
            (Some(b), Some(l)) if !l.is_empty() => {
                format!("\n- Git branch: {b}\n- Recent commits:\n{l}")
            }
            (Some(b), _) => format!("\n- Git branch: {b}"),
            _ => String::new(),
        }
    };
    let platform = std::env::consts::OS;
    let guidance = format!(
        "You are a shell expert. The user asks a natural-language question about their system \
or codebase. Determine which shell commands answer it, run them with the shell tool, then \
synthesize a clear, direct answer. Do not explain what you are about to do — just run \
commands and explain the output. Be concise.\n\
\n\
Environment:\n\
- Working directory: {cwd}\n\
- Platform: {platform}{git_ctx}"
    );
    let mut session = build_session(false, mode, false, None, None).await?;
    session
        .run_turn_with(&query, &[guidance], None)
        .await
        .context("nl query")?;
    Ok(())
}

/// Unblock a turn parked in a permission/question prompt before quitting. `Presenter::confirm`/
/// `ask` run INSIDE the turn task via `block_in_place` (a real blocking `recv()`, not an
/// `.await`), so `turn_handle.abort()` alone cannot preempt it — only dropping/answering its reply
/// channel does. Without this, quitting while such a prompt is pending deadlocks: the code right
/// after the main loop does `session.lock().await`, which blocks forever waiting for the turn task
/// to release the mutex, while that task blocks forever waiting for this reply channel — a
/// permanent hang the user can't even Ctrl-C out of (raw mode consumes it as a keystroke). Mirrors
/// the Esc/Ctrl-C interrupt cleanup; omits `loop_state`/`queued_prompts` since quitting doesn't
/// need to preserve resumable state the way interrupting-and-continuing does.
fn abort_turn_before_quit(
    turn_handle: &mut Option<tokio::task::JoinHandle<()>>,
    pending: &mut Option<(String, std::sync::mpsc::Sender<forge_tui::ConfirmOutcome>)>,
    pending_question: &mut Option<std::sync::mpsc::Sender<String>>,
    app: &mut forge_tui::App,
) {
    if let Some(h) = turn_handle.take() {
        h.abort();
    }
    *pending = None;
    *pending_question = None;
    app.prompt = None;
    app.clear_question();
}

/// Whether `prefix` is a provider Forge recognizes — a key-based/custom provider, or one of the
/// keyless ones (local `ollama`, the `claude-cli`/`codex-cli` bridges). Used to reject a clearly
/// invalid `--model provider::id` even when no catalog is available to check the full id against.
fn is_known_provider_prefix(prefix: &str) -> bool {
    // `xai-oauth` authenticates via a keyring OAuth session (`forge auth xai-oauth`), not an
    // env-var API key, so it's never in `known_key_providers()` — same reason the CLI bridges
    // are keyless here.
    const KEYLESS: &[&str] = &[
        "ollama",
        "claude-cli",
        "codex-cli",
        "xai-oauth",
        "codex-oauth",
    ];
    KEYLESS.contains(&prefix) || forge_config::known_key_providers().any(|p| p == prefix)
}

/// The OS shell used to run a `StatuslineWidget::Custom` command — same choice as hooks (see
/// `forge_core::hooks::hook_shell`), duplicated locally since it's three lines and pulling in a
/// dependency for it would be overkill.
fn shell_widget_shell() -> (&'static str, &'static str) {
    #[cfg(windows)]
    return ("cmd", "/C");
    #[cfg(not(windows))]
    ("sh", "-c")
}

/// On a fresh machine (no keys, no bridge, no config) offer the `forge init` wizard before the
/// first chat. Skipped for `--mock`, non-interactive shells, and once anything is configured.
/// Declining writes an (empty) config so we don't nag on every launch.
pub(crate) fn maybe_first_run_setup(mock: bool) -> Result<()> {
    if mock || !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return Ok(());
    }
    let has_any_key = forge_config::known_key_providers().any(forge_config::has_api_key);
    let any_bridge = forge_provider::CliKind::all().iter().any(|k| k.available());
    if !needs_onboarding(has_any_key, any_bridge, forge_config::user_config_exists()) {
        return Ok(());
    }
    println!("⚒ Welcome to Forge — no providers are configured yet.");
    let yes = prompt_line("Run guided setup now? [Y/n]: ")?;
    if yes.is_empty() || yes.eq_ignore_ascii_case("y") || yes.eq_ignore_ascii_case("yes") {
        setup()?;
    } else {
        // Mark onboarded so we don't ask again; the user can re-run `forge setup` anytime.
        let _ = forge_config::write_subscriptions(&std::collections::HashMap::new());
        println!("Skipped. Run `forge setup` anytime, or `forge auth <provider>` to add a key.");
    }
    Ok(())
}

/// Probe Claude's CURRENT rate limits (both windows, via the `claude --debug` headers) and record
/// them into the session store. Best-effort; the caller gates it on staleness. This is the live
/// claude-usage source — it replaces the helm-wiped statusline cache.
pub(crate) async fn refresh_claude_quota(session: &std::sync::Arc<tokio::sync::Mutex<Session>>) {
    let limits = tokio::task::spawn_blocking(bridge_stats::probe_claude_limits)
        .await
        .unwrap_or_default();
    if !limits.is_empty() {
        let s = session.lock().await;
        for (w, f) in limits {
            s.seed_subscription_quota("claude-cli", &w, Some(f * 100.0));
        }
    }
}

/// Whether the stored claude quota is older than `max_age` seconds (or absent) — gates the probe.
pub(crate) async fn claude_quota_is_stale(
    session: &std::sync::Arc<tokio::sync::Mutex<Session>>,
    max_age: i64,
) -> bool {
    session
        .lock()
        .await
        .claude_quota_age_secs()
        .is_none_or(|a| a > max_age)
}

pub(crate) async fn chat(
    mock: bool,
    mode: Option<Mode>,
    resume_mode: ResumeMode,
    plain: bool,
    fullscreen: bool,
    pin: Option<String>,
) -> Result<()> {
    maybe_first_run_setup(mock)?;
    maybe_autostart_local();
    // Default to the interactive (animated) TUI on a real terminal.
    if !plain && std::io::stdout().is_terminal() {
        // Update check happens in background inside run_chat_tui (via the UiMsg channel) so it
        // never delays TUI startup. The check has a 3s network timeout — blocking here would
        // freeze the terminal for up to 3s once per day.
        return run_chat_tui(mock, mode, resume_mode, fullscreen, pin).await;
    }
    // Plain path: blocking update check is fine (no TUI to corrupt).
    update_check::maybe_notify(&forge_config::load().unwrap_or_default()).await;

    // Plain line mode: read prompts from stdin.
    // Picker is already ruled out by resolve_resume_mode for headless/plain.
    let resume_id = match resume_mode {
        ResumeMode::Id(id) => Some(id),
        ResumeMode::Fresh | ResumeMode::Picker => None,
    };
    let mut session = build_session_with(
        Box::new(HeadlessPresenter::default()),
        mock,
        mode,
        resume_id,
        pin,
        false,
    )
    .await?;
    if std::io::stdin().is_terminal() {
        println!("forge chat — type a task and press enter; /quit to exit");
    }
    {
        let sid = session.session_id().to_string();
        let hooks = session.hooks().to_vec();
        forge_core::hooks::run_session_hooks(&hooks, forge_config::HookEvent::SessionStart, &sid)
            .await;
    }
    while let Some(line) = session.read_line() {
        match chat_action(&line) {
            ChatAction::Quit => break,
            ChatAction::Skip => continue,
            ChatAction::Run(task) => {
                let hooks = session.hooks().to_vec();
                let task = match forge_core::hooks::run_prompt_hooks(&hooks, &task).await {
                    Ok(t) => t,
                    Err(reason) => {
                        eprintln!("⎇ prompt blocked by hook: {reason}");
                        continue;
                    }
                };
                session
                    .run_turn(&task)
                    .await
                    .context("running agent turn")?;
            }
        }
    }
    {
        let sid = session.session_id().to_string();
        let hooks = session.hooks().to_vec();
        forge_core::hooks::run_session_hooks(&hooks, forge_config::HookEvent::SessionEnd, &sid)
            .await;
    }
    Ok(())
}

/// Sends the turn-complete signal (carrying the turn's generation) on drop — so `busy` is released
/// even if the turn task panics or is aborted. The loop only acts on a signal whose generation
/// matches the current turn, so an interrupted turn's late signal can't end a *later* turn.
pub(crate) struct DoneGuard(pub(crate) std::sync::mpsc::Sender<u64>, pub(crate) u64);

impl Drop for DoneGuard {
    fn drop(&mut self) {
        let _ = self.0.send(self.1);
    }
}

/// Animated TUI chat loop: renders at ~16fps, runs each turn on a task so a spinner
/// ticks (and streamed tokens flow) while the model works.
/// Emit pre-styled out-of-band lines to the conversation, respecting the viewport mode: inline →
/// the terminal's native scrollback; full-screen → the app's transcript log (since there's no
/// native scrollback in alternate-screen mode).
pub(crate) fn emit_scrollback(
    tui: Option<&mut forge_tui::Tui>,
    app: &mut forge_tui::App,
    lines: Vec<forge_tui::ScrollbackLine<'static>>,
) {
    match tui {
        Some(tui) if !tui.is_fullscreen() => tui.insert_lines(lines),
        // Full-screen (no native scrollback) and headless (`forge serve` — no terminal at all)
        // both land in the app's transcript log, which the remote snapshot mirrors.
        _ => app.push_scrollback(lines),
    }
}

/// Like [`emit_scrollback`] but for plain (unstyled) multi-line text.
pub(crate) fn emit_text(tui: Option<&mut forge_tui::Tui>, app: &mut forge_tui::App, text: &str) {
    match tui {
        Some(tui) if !tui.is_fullscreen() => tui.print_text(text),
        _ => app.push_scrollback_text(text),
    }
}

/// Every editable setting as `/config` editor rows, grouped: "Providers & Keys" (API keys, keyring)
/// first, then the discovered scalar settings (friendly labels, control kind, default, source).
pub(crate) fn config_editor_rows() -> Vec<forge_tui::SettingRow> {
    let mut rows: Vec<forge_tui::SettingRow> = forge_config::known_key_providers()
        .map(|p| forge_tui::SettingRow {
            path: format!("key.{p}"),
            group: "Providers & Keys".to_string(),
            label: format!("{} API key", provider_label(p)),
            help: Some(format!(
                "API key for {p}, stored in the OS keyring. Enter to set; empty to remove."
            )),
            kind: forge_tui::RowKind::Secret,
            value: if forge_config::has_api_key(p) {
                "● set".to_string()
            } else {
                "○ not set".to_string()
            },
            default: String::new(),
            modified: forge_config::has_api_key(p),
            source: "keyring".to_string(),
        })
        .collect();
    rows.extend(forge_config::config_descriptors().into_iter().map(|d| {
        let kind = match d.kind {
            forge_config::SettingKind::Bool => forge_tui::RowKind::Bool,
            forge_config::SettingKind::Int => forge_tui::RowKind::Int,
            forge_config::SettingKind::Float => forge_tui::RowKind::Float,
            forge_config::SettingKind::List => forge_tui::RowKind::Text,
            forge_config::SettingKind::Json => forge_tui::RowKind::Text,
            forge_config::SettingKind::Text => forge_tui::RowKind::Text,
            forge_config::SettingKind::Enum(opts) => {
                forge_tui::RowKind::Enum(opts.into_iter().map(str::to_string).collect())
            }
        };
        forge_tui::SettingRow {
            path: d.path,
            group: d.group,
            label: d.label,
            help: d.help,
            kind,
            value: d.value.display(),
            default: d.default.display(),
            modified: d.modified,
            source: d.source.to_string(),
        }
    }));
    // Complex sections (hooks/mcp/permissions) can't be flattened to scalars, so list them
    // read-only with an "edit in $EDITOR" jump — otherwise they're invisible in `/config`.
    rows.extend(
        forge_config::complex_sections()
            .iter()
            .map(|&section| forge_tui::SettingRow {
                path: section.to_string(),
                group: "Advanced (edit in $EDITOR)".to_string(),
                label: section.to_string(),
                help: Some(forge_config::complex_section_help(section).to_string()),
                kind: forge_tui::RowKind::ReadOnly,
                value: String::new(),
                default: String::new(),
                modified: false,
                source: "config.toml".to_string(),
            }),
    );
    rows
}

pub(crate) async fn run_chat_tui(
    mock: bool,
    mode: Option<Mode>,
    resume_mode: ResumeMode,
    fullscreen: bool,
    pin: Option<String>,
) -> Result<()> {
    use forge_tui::{
        banner_lines, handle_key, print_banner_direct, App, ChannelPresenter, ConfirmOutcome,
        InputOutcome, KeyKind, Tui, UiMsg,
    };
    use std::time::{Duration, Instant};

    let (tx, rx) = std::sync::mpsc::channel::<UiMsg>();
    let (done_tx, done_rx) = std::sync::mpsc::channel::<u64>();

    // Load config once — shared between update check, session build, and TUI config below.
    let tui_config = forge_config::load().unwrap_or_default();
    // Fire the update check in the background so it never blocks TUI startup.
    // The notification arrives as a Warning in the TUI instead of blocking on a 3s HTTP call.
    update_check::maybe_notify_background(&tui_config, tx.clone());

    // For Picker mode we start a fresh session; the picker fires on the first frame.
    let open_picker_on_start = matches!(resume_mode, ResumeMode::Picker);
    let resume_id = match &resume_mode {
        ResumeMode::Id(id) => Some(id.clone()),
        ResumeMode::Fresh | ResumeMode::Picker => None,
    };
    let tx_mcp = tx.clone(); // clone before tx is moved into ChannelPresenter
    let tx_custom = tx.clone(); // held for the render loop's shell-backed custom-widget refresh
    let session = build_session_with(
        Box::new(ChannelPresenter::new(tx)),
        mock,
        mode,
        resume_id,
        pin,
        true, // suppress initial "reconnecting" announce; re-announce fires after connect_active
    )
    .await?;
    // Grab the MCP connect-done receiver before moving the session into the Arc. When the
    // background connect_active() completes, re-announce so the TUI shows connected/failed
    // state rather than the "reconnecting" placeholder from the initial announce.
    let mcp_done_rx = session.mcp_connect_done();
    let session = std::sync::Arc::new(tokio::sync::Mutex::new(session));
    if let Some(mut rx) = mcp_done_rx {
        let s = session.clone();
        let tx2 = tx_mcp;
        tokio::spawn(async move {
            // Wait until connect_active() signals done (or 30s watchdog).
            let _ = tokio::time::timeout(std::time::Duration::from_secs(30), async {
                loop {
                    if *rx.borrow() {
                        break;
                    }
                    if rx.changed().await.is_err() {
                        break;
                    }
                }
            })
            .await;
            let status = s.lock().await.mcp_status();
            if !status.is_empty() {
                let _ = tx2.send(UiMsg::Event(forge_tui::PresenterEvent::McpStatus(status)));
            }
        });
    }

    // Seed the mesh subscription quota at startup so routing + the overlays reflect usage from
    // outside Forge. Codex comes from its rollout files (fresh); claude's stale cache is only a
    // weak fallback — the background probe below fetches claude's CURRENT 5h+weekly utilisation
    // (via the `claude --debug` rate-limit headers) so the store is live within a few seconds.
    // Skipped entirely under `--mock`: it's documented as the offline/deterministic provider, and
    // `refresh_claude_quota` spawns the REAL `claude` CLI as a subprocess — a real network call
    // with real side effects, not something "offline" should ever trigger.
    if !mock {
        // bridge_stats::fetch recursively scans ~/.claude/projects/**/*.jsonl — on a slow FS (WSL
        // /mnt, a huge history) that can stall the first frame. Run it in a background task;
        // the quota overlay refreshes on its own cadence so the numbers fill in within seconds.
        tokio::spawn({
            let s = session.clone();
            async move {
                if let Ok(bstats) = tokio::task::spawn_blocking(bridge_stats::fetch).await {
                    let sess = s.lock().await;
                    // Codex readings come from rollout files that can be hours old — seed with
                    // their true observation time so a stale re-seed can't mask fresher
                    // x-codex-header data in the shared codex quota bucket.
                    sess.seed_subscription_quota_at(
                        "codex-cli",
                        "five_hour",
                        bstats.codex_5h_pct,
                        bstats.codex_5h_observed_at,
                    );
                    sess.seed_subscription_quota_at(
                        "codex-cli",
                        "weekly",
                        bstats.codex_weekly_pct,
                        bstats.codex_weekly_observed_at,
                    );
                    sess.seed_subscription_quota("claude-cli", "five_hour", bstats.claude_5h_pct);
                    sess.seed_subscription_quota("claude-cli", "weekly", bstats.claude_weekly_pct);
                }
            }
        });
        if claude_quota_is_stale(&session, 300).await {
            tokio::spawn({
                let s = session.clone();
                async move { refresh_claude_quota(&s).await }
            });
        }
    }

    // Inline-mode banner: print BEFORE ratatui creates the inline viewport so the terminal's own
    // line-ending handling clears the full row width. ratatui's draw_lines_over_cleared uses
    // old.diff(&new) which skips default-style cells at cols 42+ (empty == empty in the diff),
    // leaving old terminal content visible to the right of the 42-char logo when the terminal has
    // prior content (e.g. Claude Code hook output). Full-screen has no scrollback; banner goes
    // into the transcript log after Tui::new().
    if matches!(resume_mode, ResumeMode::Fresh) && !fullscreen {
        print_banner_direct();
    }
    // Mouse capture (full-screen wheel scroll) is opt-in: it disables native click-drag text
    // selection, so it stays off unless the user enables `[tui] mouse_capture`.
    let mouse_capture = tui_config.tui.mouse_capture;
    let mut tui = Tui::new(fullscreen, mouse_capture, tui_config.keybinds.clone())
        .context("initializing TUI")?;
    let mut app = App::default();
    app.fullscreen = fullscreen;
    app.transcript_follow = true;
    // Wire statusline config from the loaded config.
    app.statusline_config = tui_config.statusline.clone();
    app.keybinds = tui_config.keybinds.clone();
    // Fetch current git branch for the GitBranch statusline widget (best-effort, non-fatal).
    app.git_branch = std::process::Command::new("git")
        .args(["branch", "--show-current"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stdout)
                    .ok()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
            } else {
                None
            }
        });
    // Repo/project name for the RepoName widget: the git top-level directory's name, falling back
    // to the cwd's name outside a git repo (so the widget is still useful there).
    app.repo_name = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stdout)
                    .ok()
                    .map(|s| s.trim().to_string())
            } else {
                None
            }
        })
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|p| p.to_string_lossy().into_owned())
        })
        .and_then(|p| {
            std::path::Path::new(&p)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
        });
    // Full-screen banner goes into the transcript log (no native scrollback in full-screen mode).
    // Inline banner was already printed directly to stdout above.
    if matches!(resume_mode, ResumeMode::Fresh) && fullscreen {
        let banner = banner_lines(tui.width());
        app.push_scrollback(banner);
    }
    {
        let s = session.lock().await;
        app.temper = s.temper().label().to_string();
        app.effort = s.pinned_effort();
    }

    // Populate the command palette from the skill catalog the session already loaded in
    // build_session_with — avoids a second disk scan of all skill/command dirs.
    let catalog: Arc<forge_skills::Catalog> = {
        let s = session.lock().await;
        // Reuse the Arc the session holds; fall back to a fresh load only if missing.
        s.skills().cloned().unwrap_or_else(|| {
            Arc::new(forge_skills::Catalog::load(&forge_config::command_sources()))
        })
    };
    app.palette.extra = catalog
        .entries()
        .iter()
        .map(|e| forge_tui::PaletteEntry {
            name: e.name.clone(),
            desc: if e.is_skill {
                format!("{}  (skill)", e.description)
            } else {
                e.description.clone()
            },
            // File-based commands/skills take freeform input; no fixed usage hint.
            usage: String::new(),
        })
        .collect();
    for w in catalog.warnings() {
        app.note(&format!("⚠ {w}"));
    }
    let trust_project = session.lock().await.commands_trust_project();
    // Git attribution: auto-install the model-aware commit hook when enabled, and remember the
    // flag so each turn's routed model is written where the hook can stamp it.
    let git_coauthor = tui_config.git.coauthor;
    if git_coauthor {
        maybe_install_git_hook(&tui_config);
    }
    {
        let (hooks, sid) = {
            let s = session.lock().await;
            (s.hooks().to_vec(), s.session_id().to_string())
        };
        forge_core::hooks::run_session_hooks(&hooks, forge_config::HookEvent::SessionStart, &sid)
            .await;
    }

    // On a resumed session (`--continue` / `--resume <id>`): render the FULL prior transcript into
    // scrollback (the user sees the entire original conversation, even the parts compaction folded
    // away from the model's view), then a separator marking where new input begins.
    let mut offer_resume_choice = false;
    {
        let s = session.lock().await;
        let items = s.replay_items_full();
        if !items.is_empty() {
            let sid8: String = s.session_id().chars().take(8).collect();
            let n = items.len();
            app.replay_history(&items);
            app.push_resume_separator(&format!("— resumed session {sid8} ({n} entries) —"));
            // Restore the on-screen view (activity panel, viewer, scroll) saved on the last turn,
            // so resume reopens exactly where the user left off.
            if let Some(json) = s.view_snapshot() {
                app.restore_view_json(&json);
            }
            // If this session was compacted, the model only sees a summary. Offer the choice.
            offer_resume_choice = s.was_compacted();
        }
    }

    // For bare `--resume` (Picker mode): open the session picker on the first frame so the user
    // can choose which session to reattach to. Otherwise, if we resumed a previously-compacted
    // session, ask whether to continue compacted or reload the full history into the model's view.
    if open_picker_on_start {
        open_sessions_picker(&mut app, "")?;
    } else if offer_resume_choice {
        open_resume_choice_picker(&mut app);
    }

    // Project-scope commands/skills can steer the model; their first use this session is gated
    // unless trusted. Re-running a gated command confirms it (its name lands here).
    let mut armed_project: std::collections::HashSet<String> = std::collections::HashSet::new();

    let mut busy = false;
    // Each turn gets a monotonic generation; the abort handle lets Esc interrupt it (RFC
    // session-management). The current gen gates the done-signal so an aborted turn's late
    // signal is ignored once a new turn has started.
    let mut turn_gen: u64 = 0;
    // Generation of the last auto-compact turn; prevents re-firing before a new user turn updates
    // context_tokens (compact's own Cost event still reflects the old full-context size).
    let mut last_auto_compact_gen: u64 = 0;
    let mut turn_handle: Option<tokio::task::JoinHandle<()>> = None;
    // `/loop` state: when set, each completed turn of this generation is re-run until the model
    // signals completion or the iteration cap is hit.
    let mut loop_state: Option<LoopState> = None;
    // `/goal` state: mirrors `loop_state`, driven off the tracked task plan instead of a sentinel.
    let mut goal_state: Option<GoalState> = None;
    let mut pending: Option<(String, std::sync::mpsc::Sender<ConfirmOutcome>)> = None;
    let mut pending_question: Option<std::sync::mpsc::Sender<String>> = None;
    // `/duel`: the background task writes its finished report + still-alive worktree guards here
    // (it can't return a value through `turn_handle`, a `JoinHandle<()>`); the done-signal drain
    // below takes it and opens the picker. `duel_state` then holds it across picker frames until
    // the user picks a winner (merge) or cancels (Esc discards every candidate).
    let pending_duel: Arc<std::sync::Mutex<PendingDuel>> = Arc::new(std::sync::Mutex::new(None));
    let mut duel_state: PendingDuel = None;
    // Lens filter set by `/assay --only`/`--skip`; consumed when the AssayChoice picker resolves.
    let mut assay_lenses: Vec<forge_types::FindingCategory> = Vec::new();
    // Scope set by `/assay --diff/--branch/--since/<path>`; consumed when picker resolves.
    let mut assay_scope: forge_types::AssayScope = forge_types::AssayScope::Repo;
    // Baseline for the spinner: deriving the tick from elapsed time keeps the animation
    // speed independent of the loop frequency (one frame per 60ms, exactly as before).
    let mut busy_since = Instant::now();
    // Fixed epoch for idle animations (effort slider rainbow, etc.): unlike busy_since this
    // never resets, so idle animations always have a monotonically increasing tick.
    let anim_epoch = Instant::now();
    // Receivers for overlay background loads (mesh/usage open instantly; data fills in async).
    let mut mesh_load_rx: Option<tokio::sync::oneshot::Receiver<Option<forge_tui::MeshOverlay>>> =
        None;
    let mut usage_load_rx: Option<tokio::sync::oneshot::Receiver<bridge_stats::BridgeStats>> = None;
    // `/voice`: the real system resources (recorder thread, download/transcribe channels) live
    // here, loop-local — NOT on `App`, which stays `Clone + Default` and only holds the
    // rendering-facing `VoiceOverlay`. See `apply_voice_start`/`start_voice_transcribe`.
    let mut voice_handle: Option<forge_voice::RecordingHandle> = None;
    let mut voice_model_path: Option<std::path::PathBuf> = None;
    let mut voice_download_progress_rx: Option<tokio::sync::watch::Receiver<(u64, Option<u64>)>> =
        None;
    let mut voice_download_done_rx: Option<
        tokio::sync::oneshot::Receiver<std::result::Result<forge_voice::RecordingHandle, String>>,
    > = None;
    let mut voice_transcript_rx: Option<
        tokio::sync::oneshot::Receiver<forge_voice::Result<String>>,
    > = None;
    // Wall-clock start of the current recording, for the `mm:ss` elapsed label (mirrors
    // `busy_since`/`turn_elapsed_secs`).
    let mut voice_started_at: Option<Instant> = None;
    // When set, the voice overlay's error card auto-closes at this instant (also dismissible by
    // any keypress — see the modal key handling below).
    let mut voice_error_until: Option<Instant> = None;
    // Wall-clock press time of the voice chord, for the push-to-talk tap-vs-hold decision
    // (`voice_is_hold`). Only meaningful between a `KeyKind::ToggleVoice` press and its matching
    // `InputEvent::KeyUp` release.
    let mut voice_press_at: Option<Instant> = None;
    // True while the kitty keyboard protocol's event-type reporting is pushed for push-to-talk
    // (see `Tui::push_voice_ptt`) — gates whether `Tui::pop_voice_ptt` is safe to call.
    let mut voice_ptt_active = false;
    // Remote control (`/remote`): when `Some`, a browser can drive the session. The handle owns
    // the server task + the snapshot channel + the input queue; we broadcast a snapshot each
    // dirty frame and drain inputs to inject them like local keystrokes.
    let mut remote: Option<remote::RemoteControl> = None;
    // The session id for remote snapshots — refreshed via `try_lock` in the loop (never blocking:
    // a busy turn parked in a permission/question prompt holds `session`'s lock for the ENTIRE
    // turn, and this id practically never changes mid-turn anyway, so a stale reuse costs nothing).
    let mut cached_session_id = session.lock().await.session_id().to_string();
    // The store seam behind the remote server's `GET /api/history` scrollback pagination
    // (docs/features/remote-control.md §2b): reads persisted transcript pages for whatever
    // session id the latest snapshot carries (it follows `/new`/resume automatically). A
    // closure so `remote.rs` needn't depend on forge-store.
    let remote_history: remote::HistoryProvider = {
        let store = session.lock().await.store.clone();
        std::sync::Arc::new(move |sid: &str, before: Option<i64>, limit: usize| {
            store
                .load_history_page(sid, before, limit)
                .unwrap_or_default()
                .into_iter()
                .map(|r| remote::HistoryRow {
                    seq: r.seq,
                    role: r.role.as_str().to_string(),
                    content: r.content,
                    model: r.model,
                    created_at: r.created_at,
                    visibility: r.visibility.as_str().to_string(),
                })
                .collect()
        })
    };
    // Mirrors `Session::pinned_tier` so tier_up/tier_down can read + update it WITHOUT the session
    // lock — `spawn_turn_with`'s task holds that lock for the entire turn (every provider
    // round-trip, every tool call), not just during a permission/question prompt, so `try_lock`
    // fails almost the whole time a turn is busy. The session-side field is still updated (via a
    // fire-and-forget spawn) so it stays correct for anything else that reads it directly.
    let mut local_pinned_tier: Option<forge_types::TaskTier> = None;
    // Models the user has explicitly skipped via Ctrl+K DURING THE CURRENT prompt's retry chain —
    // reset the moment a genuinely new prompt is submitted (not on a skip-triggered retry).
    // `bench_for`'s cooldown is time-based and can expire mid-chain if the user cycles through
    // several models; this set makes the exclusion last exactly as long as the user is still
    // retrying the same request, independent of how much real time that takes.
    let mut skip_model_excludes: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    // Last-spawned time per shell-backed custom statusline widget (keyed by its command string —
    // see `StatuslineWidget::Custom`), so the refresh loop below spawns each on its own cadence
    // instead of every frame.
    let mut custom_widget_last_run: std::collections::HashMap<String, Instant> =
        std::collections::HashMap::new();
    // Only redraw when state actually changed: idle frames cost nothing and the whole
    // conversation isn't rebuilt 16×/sec for no reason.
    let mut dirty = true;
    let mut quit = false;
    // Drives the input-cursor blink. The cursor stays solid while the user is actively typing and
    // only begins a calm blink after a short idle gap (like Claude Code) — measured from the last
    // input event, so it never flickers mid-keystroke.
    let mut last_input_at = std::time::Instant::now();
    // Last model written to `$GIT_DIR/forge-model` for commit attribution (only when coauthor on).
    let mut last_model_written = String::new();
    let mut prompt_history: Vec<String> = Vec::new();
    let mut history_pos: Option<usize> = None;
    let mut history_draft = String::new();
    // The prompt of the turn currently running (or last run), so `tier_up`/`tier_down` can abort
    // and re-run it at the shifted routing tier. Set whenever a plain user turn is spawned.
    let mut last_prompt: Option<String> = None;
    // Prompts typed while a turn is running, queued to run one-per-turn after it finishes
    // (like Claude Code / aider). Drained in the done-handler below; cleared on interrupt.
    let mut queued_prompts: Vec<String> = Vec::new();
    // Identity of the currently pending permission/question prompt, bumped every time a new one
    // is installed. Broadcast as `Snapshot::prompt_seq`; remote Allow/Answer must echo it back,
    // and a mismatch is ignored — a stale tap can never approve a prompt it never saw.
    let mut prompt_seq: u64 = 0;
    // Remote-facing notices (`Snapshot::notes`, bounded): feedback for inputs the remote drain
    // can't fully honor (e.g. `/remote` typed on the phone, stale answers).
    let mut remote_notes: Vec<String> = Vec::new();
    // Keystrokes injected by remote clients (`RemoteInput::Key` + synthesized overlay commits),
    // drained at the HEAD of the local key loop so they run through the exact same code path a
    // local keystroke takes — identical modal routing, identical `DispatchOutcome` handling.
    let mut remote_keys: std::collections::VecDeque<forge_tui::KeyKind> =
        std::collections::VecDeque::new();
    // The last `/copy` payload while remote is on (`Snapshot::copy_text`), so the PHONE can copy
    // the text to its own clipboard — the host clipboard is unreachable from there. Cleared on
    // the next remote prompt, like `remote_notes`.
    let mut remote_copy_text: Option<String> = None;
    // Text files uploaded via POST /api/upload, waiting to ride the next remote prompt as
    // `@path` mentions (images go straight to `Session::attach_images` instead).
    let mut remote_attach_mentions: Vec<String> = Vec::new();
    // Change-only broadcast: the last snapshot actually sent + its revision. While busy the loop
    // spins every 16ms; without this compare every iteration pushed an identical frame to every
    // connected browser (~60 frames/s of JSON for nothing).
    let mut last_remote_snap: Option<remote::Snapshot> = None;
    let mut remote_revision: u64 = 0;
    // One long-lived clipboard for mouse-selection copies (see `copy_selection`). Created once so
    // arboard keeps the X11/Wayland selection alive and never logs a "dropped" warning to the TUI.
    let mut clipboard: Option<arboard::Clipboard> = arboard::Clipboard::new().ok();

    struct ObserverState {
        session_id: String,
        store: std::sync::Arc<forge_store::Store>,
        last_event_id: i64,
        last_poll: std::time::Instant,
    }
    let mut observer: Option<ObserverState> = None;

    // The cwd is stable for remote snapshots (captured once, cheap to clone each frame instead of
    // a syscall in the hot render path). The session id is NOT stable — `/new` and the "observe a
    // live session" picker mutate it in place via `reset_fresh`/`reset_resumed` without restarting
    // this loop — so it's re-read from the session at snapshot-build time instead of cached here.
    let remote_cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default();

    // Draw the first frame before the (possibly slow) auto-start below, so `[remote] auto =
    // "anywhere"` — which waits on a public tunnel to come up — doesn't leave the terminal blank.
    tui.draw(&app);

    // Default-on remote control: when `[remote] auto` is configured, start the server at chat
    // startup so the session is reachable from a phone/browser without typing `/remote` first.
    if let Some(auto) = tui_config.remote.startup_exposure() {
        toggle_remote(
            &mut remote,
            &mut app,
            &mut tui,
            auto.into(),
            &tui_config.remote,
            remote_history.clone(),
        )
        .await?;
        // A (re)started server begins from a fresh watch channel — drop the dedup state so the
        // first frame is always broadcast even if the app state hasn't changed since the last
        // server's final snapshot.
        last_remote_snap = None;
    }

    while !quit {
        if let Some(obs) = &mut observer {
            if obs.last_poll.elapsed() >= std::time::Duration::from_millis(50) {
                obs.last_poll = std::time::Instant::now();
                if let Ok(events) = obs
                    .store
                    .live_events_after(&obs.session_id, obs.last_event_id)
                {
                    for (id, json) in events {
                        obs.last_event_id = id;
                        if let Ok(ev) =
                            serde_json::from_str::<crate::live_observer::LiveEvent>(&json)
                        {
                            if let Some(pe) = crate::live_observer::live_event_to_presenter(ev) {
                                app.apply(pe);
                                dirty = true;
                            }
                        }
                    }
                }
            }
        }

        // While the in-loop activity viewer is open during a running turn, tick the elapsed-time
        // counter at 1 Hz (it shows whole seconds) and redraw only when it changes, instead of
        // forcing a full repaint every 16 ms.
        if app.viewer.is_some() && busy {
            let new_elapsed = busy_since.elapsed().as_secs();
            if new_elapsed != app.turn_elapsed_secs {
                dirty = true;
            }
        }
        if dirty {
            app.busy = busy;
            if busy {
                app.turn_elapsed_secs = busy_since.elapsed().as_secs();
            }
            tui.draw(&app);
            dirty = false;
        }

        // Drain *all* buffered keystrokes this iteration. Reading one per frame throttled
        // input. Remote-injected keys (queued by the remote drain below, processed here on the
        // next iteration) come FIRST so an overlay commit (cursor move + synthesized Enter)
        // can't be interleaved by local typing — then the terminal's own events.
        while let Some(ev) = next_input_event(&mut remote_keys, &mut tui)? {
            dirty = true;
            // Any input counts as activity: hold the cursor solid and restart the idle timer, so
            // the blink only resumes once typing pauses.
            last_input_at = std::time::Instant::now();
            app.cursor_hidden = false;

            if observer.is_some() {
                match ev {
                    forge_tui::InputEvent::Focus(gained) => {
                        app.unfocused = !gained;
                        if gained {
                            app.cursor_hidden = false;
                        }
                    }
                    forge_tui::InputEvent::Scroll { up } => {
                        const STEP: usize = 3;
                        if app.viewer.is_some() {
                            let key = if up { KeyKind::Up } else { KeyKind::Down };
                            for _ in 0..STEP {
                                app.viewer_key(key);
                            }
                        } else if app.fullscreen {
                            if up {
                                app.transcript_scroll_up(STEP);
                            } else {
                                let body = tui.height().saturating_sub(8).max(1);
                                let (_, max_scroll) = app.transcript_metrics(tui.width(), body);
                                app.transcript_scroll_down(STEP, max_scroll);
                            }
                        }
                    }
                    forge_tui::InputEvent::Key(key) => {
                        if app.workflow.open && matches!(key, KeyKind::Esc) {
                            // The observed session's workflow view is on top: Esc closes it
                            // first, not the whole observer.
                            app.workflow.open = false;
                            dirty = true;
                        } else if matches!(key, KeyKind::Esc) {
                            observer = None;
                            tui.clear_screen();
                            app.clear_transcript();
                            app.input.clear();
                            let _ = open_sessions_picker(&mut app, "");
                            dirty = true;
                        } else if app.fullscreen
                            && matches!(key, KeyKind::PageUp | KeyKind::PageDown)
                        {
                            let body = tui.height().saturating_sub(8).max(1);
                            if matches!(key, KeyKind::PageUp) {
                                app.transcript_scroll_up(body as usize);
                            } else {
                                let (_, max_scroll) = app.transcript_metrics(tui.width(), body);
                                app.transcript_scroll_down(body as usize, max_scroll);
                            }
                            dirty = true;
                        } else if app.fullscreen && matches!(key, KeyKind::JumpBottom) {
                            app.transcript_to_bottom();
                            dirty = true;
                        }
                    }
                    _ => {}
                }
                continue;
            }

            let key = match ev {
                forge_tui::InputEvent::Paste(s) => {
                    // Pasting an image: terminals deliver an empty/whitespace bracketed-paste for
                    // image clipboard content, so on an empty payload probe the OS clipboard for an
                    // image and drop it in as an attachment block. Otherwise it's a normal text paste.
                    if s.trim().is_empty() {
                        if let Some((att, label)) = crate::image_input::clipboard_image() {
                            app.attach_image(att, &label);
                            app.note(&format!("📎 attached image ({label})"));
                            continue;
                        }
                    }
                    app.handle_paste(s);
                    continue;
                }
                forge_tui::InputEvent::Focus(gained) => {
                    // Window focus changed: dim/hollow the input cursor while another window is in
                    // front, restore the solid block on return. Reset the blink phase on regain so
                    // the cursor reappears immediately rather than mid-"off" frame.
                    app.unfocused = !gained;
                    if gained {
                        app.cursor_hidden = false;
                    }
                    continue;
                }
                forge_tui::InputEvent::Scroll { up } => {
                    // Mouse wheel: the mesh inspector overlay first (it captures all input while
                    // open, same as its ↑/↓ key handling below), else the activity viewer
                    // (full-screen), else the main transcript. A few rows per notch feels natural.
                    const STEP: usize = 3;
                    if app.workflow.open {
                        let key = if up { KeyKind::Up } else { KeyKind::Down };
                        if app.workflow.zoom.is_some() {
                            for _ in 0..STEP {
                                workflow_zoom_key(&mut app, key);
                            }
                        } else {
                            app.workflow.move_selection(if up { -1 } else { 1 });
                        }
                    } else if app.mesh_overlay.open {
                        let max = app.mesh_overlay.candidates.len().saturating_sub(1);
                        for _ in 0..STEP {
                            if up {
                                app.mesh_overlay.cursor = app.mesh_overlay.cursor.saturating_sub(1);
                            } else {
                                app.mesh_overlay.cursor = (app.mesh_overlay.cursor + 1).min(max);
                            }
                        }
                    } else if app.viewer.is_some() {
                        let key = if up { KeyKind::Up } else { KeyKind::Down };
                        for _ in 0..STEP {
                            app.viewer_key(key);
                        }
                    } else if app.fullscreen {
                        if up {
                            app.transcript_scroll_up(STEP);
                        } else {
                            let body = tui.height().saturating_sub(8).max(1);
                            let (_, max_scroll) = app.transcript_metrics(tui.width(), body);
                            app.transcript_scroll_down(STEP, max_scroll);
                        }
                    }
                    continue;
                }
                forge_tui::InputEvent::Mouse { kind, col, row } => {
                    // Full-screen mouse: drag to select text (copied on release), click the floating
                    // jump-to-bottom bar. Only meaningful in the transcript (not the activity viewer).
                    use forge_tui::MouseKind;
                    if app.fullscreen && app.viewer.is_none() {
                        match kind {
                            MouseKind::Down => {
                                if app.jump_bar_hit(col, row) {
                                    app.transcript_to_bottom();
                                } else {
                                    app.clear_selection();
                                    app.selection_begin(col, row);
                                }
                            }
                            MouseKind::Drag => app.selection_extend(col, row),
                            MouseKind::Up => {
                                if let Some(text) = app.selection_text() {
                                    copy_selection(&mut clipboard, &text);
                                }
                            }
                        }
                    }
                    continue;
                }
                forge_tui::InputEvent::KeyUp(kind) => {
                    // Release of the `/voice` push-to-talk chord (the only chord this ever fires
                    // for — see `InputEvent::KeyUp`'s doc comment). A hold at/past the threshold
                    // auto-stops and transcribes; a quick tap leaves the overlay in ordinary
                    // toggle mode (Enter/Esc/r), untouched.
                    if matches!(kind, KeyKind::ToggleVoice) {
                        if let Some(pressed_at) = voice_press_at.take() {
                            let is_recording = matches!(
                                app.voice.as_ref().map(|v| &v.phase),
                                Some(forge_tui::VoicePhase::Recording { .. })
                            );
                            if is_recording && voice_is_hold(pressed_at.elapsed().as_millis()) {
                                if let (Some(handle), Some(model_path)) =
                                    (voice_handle.take(), voice_model_path.clone())
                                {
                                    if voice_ptt_active {
                                        tui.pop_voice_ptt();
                                        voice_ptt_active = false;
                                    }
                                    voice_started_at = None;
                                    voice_transcript_rx =
                                        Some(start_voice_transcribe(&mut app, handle, model_path));
                                    dirty = true;
                                }
                            }
                        }
                    }
                    continue;
                }
                forge_tui::InputEvent::Key(k) => k,
            };

            // The workflow view is modal while open: ↑↓ move the row selection, Enter zooms into
            // the selected agent's transcript (Esc steps back out), Esc/q background the view —
            // the script keeps running and the one-line status band takes over above the input.
            if app.workflow.open {
                if app.workflow.zoom.is_some() {
                    workflow_zoom_key(&mut app, key);
                } else {
                    match key {
                        KeyKind::Esc | KeyKind::Char('q') => app.workflow.open = false,
                        KeyKind::Up | KeyKind::Char('k') => app.workflow.move_selection(-1),
                        KeyKind::Down | KeyKind::Char('j') => app.workflow.move_selection(1),
                        KeyKind::PageUp => app.workflow.move_selection(-5),
                        KeyKind::PageDown => app.workflow.move_selection(5),
                        KeyKind::Home => app.workflow.selected = 0,
                        KeyKind::End => {
                            app.workflow.selected = app.workflow.rows.len().saturating_sub(1);
                        }
                        KeyKind::Enter if !app.workflow.rows.is_empty() => {
                            app.workflow.zoom = Some(Default::default());
                        }
                        _ => {}
                    }
                }
                dirty = true;
                continue;
            }

            // The in-loop activity viewer (full-screen mode) is modal while open: it owns every key
            // (scroll / switch entry / Esc to close). Rendered through the main terminal, so there's
            // no nested alternate screen to collide with the chat.
            if app.viewer_key(key) {
                dirty = true;
                continue;
            }

            // The `/config` editor is modal while open: it owns every key (filter / navigate / edit
            // / Tab scope / Esc). The editor returns an action; the shell performs the validated
            // write and refreshes the rows.
            if app.config_editor.open {
                match app.config_editor.handle_key(key) {
                    forge_tui::ConfigAction::Save { path, value } => {
                        let result = if let Some(provider) = path.strip_prefix("key.") {
                            // Secret: store/remove the API key in the OS keyring (never config.toml).
                            if value.trim().is_empty() {
                                forge_config::remove_api_key(provider)
                                    .map(|_| ())
                                    .map_err(|e| e.to_string())
                            } else {
                                forge_config::store_api_key(provider, value.trim())
                                    .map_err(|e| e.to_string())
                            }
                        } else {
                            let scope = if app.config_editor.project_scope {
                                forge_config::ConfigScope::Project
                            } else {
                                forge_config::ConfigScope::User
                            };
                            forge_config::set_config_value(scope, &path, &value)
                                .map_err(|e| e.to_string())
                        };
                        match result {
                            Ok(()) => {
                                app.config_editor.rows = config_editor_rows();
                                app.config_editor.status = Some(format!("✓ saved {path}"));
                            }
                            Err(e) => app.config_editor.status = Some(format!("✗ {e}")),
                        }
                    }
                    forge_tui::ConfigAction::Reset { path } => {
                        let scope = if app.config_editor.project_scope {
                            forge_config::ConfigScope::Project
                        } else {
                            forge_config::ConfigScope::User
                        };
                        match forge_config::reset_config_value(scope, &path) {
                            Ok(()) => {
                                app.config_editor.rows = config_editor_rows();
                                app.config_editor.status =
                                    Some(format!("✓ reset {path} to default"));
                            }
                            Err(e) => app.config_editor.status = Some(format!("✗ {e}")),
                        }
                    }
                    forge_tui::ConfigAction::Reload => {
                        app.config_editor.rows = config_editor_rows();
                    }
                    forge_tui::ConfigAction::EditFile => {
                        // Jump to $EDITOR for a complex section (hooks/mcp/permissions). Tear down +
                        // rebuild the terminal around the editor (same primitive as the keybind
                        // configurator), then reload rows to reflect any edits.
                        let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
                        if let Some(dir) = forge_config::config_dir() {
                            let path = dir.join("config.toml");
                            let _ = tui.run_fullscreen(|| {
                                std::process::Command::new(&editor)
                                    .arg(&path)
                                    .status()
                                    .map(|_| ())
                            });
                            app.config_editor.rows = config_editor_rows();
                            app.config_editor.status = Some(format!("edited {}", path.display()));
                        } else {
                            app.config_editor.status =
                                Some("⚠ no config directory on this platform".to_string());
                        }
                    }
                    forge_tui::ConfigAction::Close | forge_tui::ConfigAction::None => {}
                }
                dirty = true;
                continue;
            }

            // Effort slider is modal while open: ←/→ adjust level, Esc/Enter/Ctrl+R close.
            // `try_lock` on the writes below (not a blocking `.await`): same reasoning as
            // skip_model — a turn's task holds the session lock for its ENTIRE duration (every
            // provider round-trip, every tool call), not just during a permission/question prompt,
            // so this must apply the effort change even while a turn is busy: `try_lock` when
            // available (always true when idle), else a fire-and-forget spawn. The slider always
            // moves visually regardless — `app.effort` is local UI state, not read from `session`.
            if app.effort_slider {
                match key {
                    KeyKind::Left => {
                        app.effort_slider_left();
                        if let Some(level) = app.effort {
                            if let Ok(mut s) = session.try_lock() {
                                s.set_effort(Some(level));
                            } else {
                                let s = session.clone();
                                tokio::spawn(async move { s.lock().await.set_effort(Some(level)) });
                            }
                        }
                    }
                    KeyKind::Right => {
                        app.effort_slider_right();
                        if let Some(level) = app.effort {
                            if let Ok(mut s) = session.try_lock() {
                                s.set_effort(Some(level));
                            } else {
                                let s = session.clone();
                                tokio::spawn(async move { s.lock().await.set_effort(Some(level)) });
                            }
                        }
                    }
                    KeyKind::Esc | KeyKind::Enter | KeyKind::ToggleEffortSlider => {
                        app.effort_slider = false;
                    }
                    _ => {}
                }
                dirty = true;
                continue;
            }

            // The palette/pickers/overlays below all document themselves as "captures all keys
            // while open" (Esc closes, ↑/↓ navigate, etc.) — but every global hotkey between here
            // and there previously fired unconditionally on `key`, reaching PAST an open modal and
            // mutating turn/session state it never expected (e.g. Ctrl-K aborted the running turn
            // while /mesh was open instead of being swallowed by the overlay; the palette's own
            // match arm even has a dedicated no-op case for these keys that this ordering made
            // unreachable dead code). Skip straight past all of them while any modal is open, so
            // control reaches that modal's own handler further down instead.
            let any_modal_open = app.palette.open
                || app.usage_overlay.open
                || app.mesh_overlay.open
                || app.at_picker.open
                || app.picker.open
                || app.voice.is_some();
            if !any_modal_open {
                // Ctrl+R: toggle the effort slider when nothing else is modal.
                if matches!(key, KeyKind::ToggleEffortSlider) {
                    app.toggle_effort_slider();
                    dirty = true;
                    continue;
                }

                // Global action keys — work in all states (busy and idle).
                if matches!(key, KeyKind::ToggleReasoning) {
                    app.show_thinking = !app.show_thinking;
                    let state = if app.show_thinking { "on" } else { "off" };
                    app.note(&format!("reasoning display {state}"));
                    dirty = true;
                    continue;
                }

                if matches!(key, KeyKind::OpenKeybindConfig) {
                    let result = tui
                        .run_fullscreen(|| forge_tui::run_keybind_configurator(&mut app.keybinds));
                    if let Ok(true) = result {
                        // Save changed binds
                        let defaults = forge_config::KeybindsConfig::default();
                        for (action, combo) in &app.keybinds.binds {
                            if defaults.binds.get(action.as_str()) != Some(combo) {
                                let _ = forge_config::write_keybind(action, combo);
                            }
                        }
                        tui.keybinds = app.keybinds.clone();
                        app.note("✓ keybinds saved");
                    }
                    dirty = true;
                    continue;
                }

                if matches!(key, KeyKind::ShowHelp) {
                    let bindings = app.keybinds.clone();
                    let _ = tui.run_fullscreen(|| forge_tui::run_help(&bindings));
                    dirty = true;
                    continue;
                }

                // `?` on an empty, idle prompt opens the keybind help (discoverability) — once the user
                // has typed anything, `?` is a literal character and falls through to input handling.
                if matches!(key, KeyKind::Char('?'))
                    && app.input.is_empty()
                    && !busy
                    && !app.palette.open
                {
                    let bindings = app.keybinds.clone();
                    let _ = tui.run_fullscreen(|| forge_tui::run_help(&bindings));
                    dirty = true;
                    continue;
                }

                if matches!(key, KeyKind::EffortCycle) {
                    use forge_types::EffortLevel;
                    let next = match app.effort {
                        None => Some(EffortLevel::Low),
                        Some(EffortLevel::Low) => Some(EffortLevel::Medium),
                        Some(EffortLevel::Medium) => Some(EffortLevel::High),
                        Some(EffortLevel::High) => Some(EffortLevel::XHigh),
                        Some(EffortLevel::XHigh) => Some(EffortLevel::WhiteHot),
                        Some(EffortLevel::WhiteHot) => None,
                    };
                    app.effort = next;
                    // `try_lock` when available (idle), else fire-and-forget — same reasoning as
                    // tier_up above (a busy turn's task holds the lock for its whole duration).
                    if let Ok(mut s) = session.try_lock() {
                        s.set_effort(next);
                    } else {
                        let s = session.clone();
                        tokio::spawn(async move { s.lock().await.set_effort(next) });
                    }
                    let label = match next {
                        None => "unset (provider default)".to_string(),
                        Some(l) => format!("{l:?}").to_lowercase(),
                    };
                    app.note(&format!("effort → {label}"));
                    dirty = true;
                    continue;
                }

                if matches!(key, KeyKind::CopyLast) {
                    // Copy last assistant response to clipboard — equivalent to /copy.
                    match dispatch_command(
                        "/copy",
                        &session,
                        Some(&mut tui),
                        &mut app,
                        &catalog,
                        &mut armed_project,
                        trust_project,
                        busy,
                        &mut assay_lenses,
                        &mut assay_scope,
                    )
                    .await?
                    {
                        DispatchOutcome::CopyToClipboard(text) => {
                            let chars = text.chars().count();
                            copy_selection(&mut clipboard, &text);
                            app.note(&format!("✓ copied last response ({chars} chars)"));
                            if remote.is_some() {
                                remote_copy_text = Some(text);
                            }
                        }
                        DispatchOutcome::Quit => {
                            abort_turn_before_quit(
                                &mut turn_handle,
                                &mut pending,
                                &mut pending_question,
                                &mut app,
                            );
                            quit = true;
                            break;
                        }
                        _ => {}
                    }
                    dirty = true;
                    continue;
                }

                if matches!(key, KeyKind::ReloadConfig) {
                    if let Ok(cfg) = forge_config::load() {
                        app.keybinds = cfg.keybinds.clone();
                        tui.keybinds = cfg.keybinds;
                        // Clear the tier override so a reload returns to normal routing — see
                        // `local_pinned_tier`'s declaration comment for why this prefers `try_lock`
                        // over a blocking `.await`.
                        local_pinned_tier = None;
                        if let Ok(mut s) = session.try_lock() {
                            s.pin_tier(None);
                        } else {
                            let s = session.clone();
                            tokio::spawn(async move { s.lock().await.pin_tier(None) });
                        }
                        app.note("✓ config reloaded (tier override cleared)");
                    } else {
                        app.note("⚠ config reload failed");
                    }
                    dirty = true;
                    continue;
                }

                if matches!(key, KeyKind::ToggleVoice) {
                    // Ctrl+V (configurable): same as typing `/voice` — open the recording overlay
                    // (or kick off the whisper model download first, on first use). Not gated on
                    // `busy` here: `dispatch_command`'s own `mutates` check excludes `Voice`
                    // (like `/usage`, it only fills the input box on completion — it never touches
                    // the running turn). `voice_press_at` feeds the push-to-talk tap-vs-hold
                    // decision on the matching `InputEvent::KeyUp` release, if one ever arrives.
                    voice_press_at = Some(Instant::now());
                    if let DispatchOutcome::PendingVoice(start) = dispatch_command(
                        "/voice",
                        &session,
                        Some(&mut tui),
                        &mut app,
                        &catalog,
                        &mut armed_project,
                        trust_project,
                        busy,
                        &mut assay_lenses,
                        &mut assay_scope,
                    )
                    .await?
                    {
                        apply_voice_start(
                            start,
                            &mut tui,
                            &mut app,
                            &mut voice_handle,
                            &mut voice_model_path,
                            &mut voice_download_progress_rx,
                            &mut voice_download_done_rx,
                            &mut voice_started_at,
                            &mut voice_error_until,
                            &mut voice_ptt_active,
                        );
                    }
                    dirty = true;
                    continue;
                }

                // Ctrl-↑ / Ctrl-↓ (tier_up / tier_down): bias the mesh routing tier for the next turn.
                // The baseline is the tier currently shown in the statusline (the last routed/classified
                // tier), defaulting to Standard. Mid-turn it aborts and re-runs the same prompt at the
                // shifted tier; idle it sets the override so the next turn routes there. Clamped at the
                // ends. The override persists until changed again or `/reload`.
                if matches!(key, KeyKind::TierUp | KeyKind::TierDown) {
                    let up = matches!(key, KeyKind::TierUp);
                    let baseline = app
                        .routing
                        .as_ref()
                        .and_then(|r| forge_types::TaskTier::from_name(&r.tier))
                        .unwrap_or(forge_types::TaskTier::Standard);
                    // Computed from `local_pinned_tier`, NOT `session.lock()` — a running turn's
                    // task holds that lock for its ENTIRE duration (every provider round-trip,
                    // every tool call — see `local_pinned_tier`'s declaration comment), so this
                    // must work the whole time a turn is busy, not just when it happens to be idle.
                    let before = local_pinned_tier.unwrap_or(baseline);
                    let clamped = (up && before.is_max()) || (!up && before.is_min());
                    if clamped {
                        // Already at the extreme — report it without churning a re-run.
                        let edge = if up { "complex" } else { "trivial" };
                        app.note(&format!("⤒ already at the {edge} tier"));
                        dirty = true;
                        continue;
                    }
                    let new_tier = if up { before.up() } else { before.down() };
                    local_pinned_tier = Some(new_tier);
                    // Persist to the session for anything that reads it directly (e.g. a plain-text
                    // prompt submitted right after this, or `/reload`). `try_lock` succeeds
                    // immediately when idle (nothing else holds the lock then) — the only case
                    // where a subsequent new prompt could race a fire-and-forget write. When busy,
                    // fall back to fire-and-forget: the mid-turn retry below passes `new_tier`
                    // straight to `spawn_turn_with` and never needs to read it back anyway.
                    if let Ok(mut s) = session.try_lock() {
                        s.pin_tier(Some(new_tier));
                    } else {
                        let s = session.clone();
                        tokio::spawn(async move { s.lock().await.pin_tier(Some(new_tier)) });
                    }
                    let arrow = if up { "↑" } else { "↓" };
                    if busy {
                        // Mid-turn: abort and re-run the same prompt at the new tier.
                        if let Some(h) = turn_handle.take() {
                            h.abort();
                        }
                        turn_gen += 1;
                        busy = false;
                        loop_state = None;
                        goal_state = None;
                        pending = None;
                        pending_question = None;
                        app.prompt = None;
                        app.clear_question();
                        app.apply(forge_tui::PresenterEvent::AssistantDone);
                        if let Some(p) = last_prompt.clone() {
                            app.note(&format!(
                                "{arrow} tier → {} — re-running at the new tier",
                                new_tier.as_str()
                            ));
                            turn_gen += 1;
                            turn_handle = Some(spawn_turn_with(
                                p,
                                Vec::new(),
                                Some(new_tier),
                                &session,
                                &done_tx,
                                turn_gen,
                                &mut app,
                                &mut busy,
                                &mut busy_since,
                            ));
                        } else {
                            app.note(&format!(
                                "{arrow} tier → {} (applies to your next turn)",
                                new_tier.as_str()
                            ));
                        }
                    } else {
                        app.note(&format!(
                            "{arrow} tier → {} (applies to your next turn)",
                            new_tier.as_str()
                        ));
                    }
                    dirty = true;
                    continue;
                }

                // Ctrl-K (skip_model): mid-turn, abort + immediately retry the SAME prompt on the
                // mesh's next fallback model — the current model is briefly benched (the same
                // mechanism a real rate-limit uses, see `advance_fallback`) so normal routing
                // naturally skips it and picks the next-best candidate in the proper mesh order,
                // rather than requiring a manual `/model` pick. Idle it's a no-op note.
                if matches!(key, KeyKind::SkipModel) {
                    if busy {
                        let skipped = app.routing.as_ref().map(|r| r.model.clone());
                        if let Some(m) = &skipped {
                            skip_model_excludes.insert(m.clone());
                        }
                        if let Some(h) = turn_handle.take() {
                            h.abort();
                        }
                        turn_gen += 1;
                        busy = false;
                        loop_state = None;
                        goal_state = None;
                        if !queued_prompts.is_empty() {
                            queued_prompts.clear();
                            app.set_queued(&queued_prompts);
                        }
                        pending = None;
                        pending_question = None;
                        app.prompt = None;
                        app.clear_question();
                        app.apply(forge_tui::PresenterEvent::AssistantDone);
                        if let Some(p) = last_prompt.clone() {
                            app.note(&match &skipped {
                                Some(m) => format!("⏭ skipped {m} — retrying on next model"),
                                None => "⏭ retrying on next model".to_string(),
                            });
                            turn_gen += 1;
                            // A custom spawn (not `spawn_turn_with`): the exclusions MUST be
                            // benched inside the SAME lock acquisition that `run_turn_with` uses
                            // to classify/route, immediately before it — not from the render loop
                            // via `try_lock`, which raced the old (aborted, but not yet actually
                            // torn down — `abort()` is cooperative, not immediate) turn task for
                            // the same lock and lost almost every time, silently dropping the
                            // exclusion and landing back on the very model just skipped.
                            app.on_turn_start();
                            app.submit_user(&p);
                            app.done = false;
                            app.tick = 0;
                            busy = true;
                            busy_since = std::time::Instant::now();
                            let s = session.clone();
                            let dt = done_tx.clone();
                            let excludes: Vec<String> =
                                skip_model_excludes.iter().cloned().collect();
                            let gen = turn_gen;
                            turn_handle = Some(tokio::spawn(async move {
                                let _done = DoneGuard(dt, gen);
                                let mut sess = s.lock().await;
                                for m in &excludes {
                                    let _ = sess.store.bench_for(
                                        m,
                                        std::time::Duration::from_secs(180),
                                        "user skipped via Ctrl+K",
                                    );
                                }
                                if let Err(e) = sess.run_turn_with(&p, &[], None).await {
                                    sess.notify_error(&format!("turn failed: {e}"));
                                }
                            }));
                        } else {
                            app.note("⏭ turn aborted — no prompt to retry");
                        }
                    } else {
                        app.note("⏭ skip-model only applies mid-turn");
                    }
                    dirty = true;
                    continue;
                }
            } // !any_modal_open

            // The command palette is modal while open: it owns every key. Esc dismisses it
            // (so the user isn't surprised by a quit); Ctrl-C still maps to Esc → here it just
            // closes the palette, and a second Esc with the palette closed quits as usual.
            if app.palette.open {
                match key {
                    KeyKind::Esc => {
                        app.palette.close();
                        app.input.clear();
                    }
                    KeyKind::Up => app.palette.move_up(),
                    KeyKind::Down => app.palette.move_down(),
                    KeyKind::Tab => {
                        if let Some(name) = app.palette.selected_name().map(|s| s.to_string()) {
                            // Replace the `/command` token in place (mid-line aware), not the
                            // whole input — so `run /or<Tab>` completes to `run /orchestrate`.
                            if let Some(tok) = forge_tui::slash_token_at(
                                &app.input,
                                app.input_cursor.min(app.input.len()),
                            ) {
                                app.input
                                    .replace_range(tok.start..tok.end, &format!("/{name}"));
                                app.input_cursor = app.input.len();
                            } else {
                                app.input = format!("/{name}");
                                app.input_cursor = app.input.len();
                            }
                            app.palette.query = name;
                            app.palette.clamp();
                        }
                    }
                    KeyKind::Enter => {
                        let leading = app.input.starts_with('/') && !app.input.starts_with("//");
                        if !leading {
                            // Mid-line `/command`: Enter accepts the highlighted suggestion in
                            // place (replacing just the token) and keeps editing — it does NOT
                            // dispatch, so the surrounding prose is preserved. A leading command
                            // still dispatches (the branch below).
                            if let Some(name) = app.palette.selected_name().map(|s| s.to_string()) {
                                if let Some(tok) = forge_tui::slash_token_at(
                                    &app.input,
                                    app.input_cursor.min(app.input.len()),
                                ) {
                                    app.input
                                        .replace_range(tok.start..tok.end, &format!("/{name}"));
                                    app.input_cursor = app.input.len();
                                }
                            }
                            app.palette.close();
                            continue;
                        }
                        // If the user typed args after the command, dispatch exactly what they
                        // wrote (`/loop do it`); only autocomplete-to-selection when the line is
                        // the bare command token, so args are never dropped.
                        let has_args = app.input.trim().contains(char::is_whitespace);
                        let line = if has_args {
                            app.input.clone()
                        } else {
                            app.palette
                                .selected_name()
                                .map(|n| format!("/{n}"))
                                .unwrap_or_else(|| app.input.clone())
                        };
                        app.palette.close();
                        app.input.clear();
                        match dispatch_command(
                            &line,
                            &session,
                            Some(&mut tui),
                            &mut app,
                            &catalog,
                            &mut armed_project,
                            trust_project,
                            busy,
                            &mut assay_lenses,
                            &mut assay_scope,
                        )
                        .await?
                        {
                            DispatchOutcome::Quit => {
                                abort_turn_before_quit(
                                    &mut turn_handle,
                                    &mut pending,
                                    &mut pending_question,
                                    &mut app,
                                );
                                quit = true;
                                break;
                            }
                            DispatchOutcome::Handled => {}
                            DispatchOutcome::RunTurn {
                                prompt,
                                guidance,
                                tier,
                            } => {
                                turn_gen += 1;
                                turn_handle = Some(spawn_turn_with(
                                    prompt,
                                    guidance,
                                    tier,
                                    &session,
                                    &done_tx,
                                    turn_gen,
                                    &mut app,
                                    &mut busy,
                                    &mut busy_since,
                                ));
                            }
                            DispatchOutcome::RunCompact => {
                                turn_gen += 1;
                                turn_handle = Some(spawn_compact(
                                    &session,
                                    &done_tx,
                                    turn_gen,
                                    &mut app,
                                    &mut busy,
                                    &mut busy_since,
                                ));
                            }
                            DispatchOutcome::RunSavedWorkflow { name, args } => {
                                turn_gen += 1;
                                turn_handle = Some(spawn_saved_workflow(
                                    &session,
                                    &done_tx,
                                    turn_gen,
                                    &mut app,
                                    &mut busy,
                                    &mut busy_since,
                                    name,
                                    args,
                                ));
                            }
                            DispatchOutcome::RunDuel { task } => {
                                turn_gen += 1;
                                turn_handle = Some(spawn_duel(
                                    &session,
                                    &done_tx,
                                    turn_gen,
                                    &mut app,
                                    &mut busy,
                                    &mut busy_since,
                                    task,
                                    Arc::clone(&pending_duel),
                                ));
                            }
                            DispatchOutcome::StartLoop { prompt } => {
                                turn_gen += 1;
                                loop_state = Some(LoopState {
                                    gen: turn_gen,
                                    iter: 1,
                                });
                                app.note("↻ loop started — Esc to stop");
                                turn_handle = Some(spawn_turn_with(
                                    prompt,
                                    vec![LOOP_GUIDANCE.to_string()],
                                    None,
                                    &session,
                                    &done_tx,
                                    turn_gen,
                                    &mut app,
                                    &mut busy,
                                    &mut busy_since,
                                ));
                            }
                            DispatchOutcome::StartGoal { prompt, goal } => {
                                turn_gen += 1;
                                goal_state = Some(GoalState {
                                    gen: turn_gen,
                                    iter: 1,
                                    prev_done: 0,
                                    no_progress: 0,
                                    goal,
                                });
                                app.note("🎯 goal started — Esc to stop");
                                turn_handle = Some(spawn_turn_with(
                                    prompt,
                                    vec![GOAL_GUIDANCE.to_string()],
                                    Some(forge_types::TaskTier::Complex),
                                    &session,
                                    &done_tx,
                                    turn_gen,
                                    &mut app,
                                    &mut busy,
                                    &mut busy_since,
                                ));
                            }
                            DispatchOutcome::PendingMesh(rx) => {
                                mesh_load_rx = Some(rx);
                            }
                            DispatchOutcome::PendingUsage(rx) => {
                                usage_load_rx = Some(rx);
                            }
                            DispatchOutcome::PendingVoice(start) => {
                                apply_voice_start(
                                    start,
                                    &mut tui,
                                    &mut app,
                                    &mut voice_handle,
                                    &mut voice_model_path,
                                    &mut voice_download_progress_rx,
                                    &mut voice_download_done_rx,
                                    &mut voice_started_at,
                                    &mut voice_error_until,
                                    &mut voice_ptt_active,
                                );
                            }
                            DispatchOutcome::ToggleRemote { exposure } => {
                                toggle_remote(
                                    &mut remote,
                                    &mut app,
                                    &mut tui,
                                    exposure,
                                    &tui_config.remote,
                                    remote_history.clone(),
                                )
                                .await?;
                                // Fresh server, fresh watch channel — reset the change-only
                                // broadcast dedup so the first frame always goes out.
                                last_remote_snap = None;
                            }
                            DispatchOutcome::CopyToClipboard(text) => {
                                let chars = text.chars().count();
                                copy_selection(&mut clipboard, &text);
                                app.note(&format!(
                                    "✓ copied response to clipboard ({chars} chars)"
                                ));
                                if remote.is_some() {
                                    remote_copy_text = Some(text);
                                }
                            }
                        }
                    }
                    KeyKind::CycleTemper
                    | KeyKind::ToggleSubagentDetail
                    | KeyKind::ToggleEffortSlider
                    | KeyKind::SkipModel
                    | KeyKind::TierUp
                    | KeyKind::TierDown
                    | KeyKind::ToggleReasoning
                    | KeyKind::OpenKeybindConfig
                    | KeyKind::ModelPicker
                    | KeyKind::EffortCycle
                    | KeyKind::TemperCycle
                    | KeyKind::CopyLast
                    | KeyKind::ShowHelp
                    | KeyKind::SaveCheckpoint
                    | KeyKind::NewSession
                    | KeyKind::UndoWrite
                    | KeyKind::CompactSession
                    | KeyKind::ReloadConfig => {}
                    // Any other editing key mutates the input at the *cursor* (not blindly at the
                    // end) and then re-syncs the palette to the slash-token the cursor now sits in.
                    // That keeps the text cursor moving while the palette is open, and closes the
                    // palette once the cursor leaves the command name (e.g. a space into the args).
                    _ => {
                        let _ = forge_tui::handle_key(&mut app.input, &mut app.input_cursor, key);
                        sync_palette_to_slash_token(&mut app);
                    }
                }
                continue;
            }

            // Usage overlay captures all keys; Esc closes it.
            if app.usage_overlay.open {
                if matches!(key, KeyKind::Esc) {
                    app.usage_overlay.open = false;
                    dirty = true;
                }
                continue;
            }

            // Mesh inspector overlay captures all keys; Esc closes, ↑/↓ move the candidate cursor
            // (browsing highlight, independent of the actual routed pick — see MeshOverlay::cursor).
            if app.mesh_overlay.open {
                match key {
                    KeyKind::Esc => {
                        app.mesh_overlay.open = false;
                        app.mesh_overlay.cursor = 0;
                        dirty = true;
                    }
                    KeyKind::Down => {
                        let max = app.mesh_overlay.candidates.len().saturating_sub(1);
                        app.mesh_overlay.cursor = (app.mesh_overlay.cursor + 1).min(max);
                        dirty = true;
                    }
                    KeyKind::Up => {
                        app.mesh_overlay.cursor = app.mesh_overlay.cursor.saturating_sub(1);
                        dirty = true;
                    }
                    _ => {}
                }
                continue;
            }

            // The `/voice` recording overlay captures all keys while open. Match on a cloned
            // phase (not a live borrow) since several arms need to mutate `app.voice` itself.
            if let Some(phase) = app.voice.as_ref().map(|v| v.phase.clone()) {
                match phase {
                    forge_tui::VoicePhase::Error(_) => {
                        // Any key dismisses early; it also auto-closes after ~2s (ticked below).
                        app.voice = None;
                        voice_error_until = None;
                        dirty = true;
                    }
                    forge_tui::VoicePhase::Downloading { .. } => {
                        if matches!(key, KeyKind::Esc) {
                            // `forge_voice::ensure_model` has no cancel token — the download
                            // finishes in the background and its result is simply never looked
                            // at again (the receivers below are dropped).
                            app.voice = None;
                            voice_download_progress_rx = None;
                            voice_download_done_rx = None;
                            app.note("voice: cancelled");
                            dirty = true;
                        }
                    }
                    forge_tui::VoicePhase::Recording { .. } => match key {
                        KeyKind::Esc => {
                            if let Some(h) = voice_handle.take() {
                                h.cancel();
                            }
                            if voice_ptt_active {
                                tui.pop_voice_ptt();
                                voice_ptt_active = false;
                            }
                            voice_started_at = None;
                            voice_model_path = None;
                            app.voice = None;
                            app.note("voice: cancelled");
                            dirty = true;
                        }
                        KeyKind::Enter => {
                            if let (Some(handle), Some(model_path)) =
                                (voice_handle.take(), voice_model_path.clone())
                            {
                                if voice_ptt_active {
                                    tui.pop_voice_ptt();
                                    voice_ptt_active = false;
                                }
                                voice_started_at = None;
                                voice_transcript_rx =
                                    Some(start_voice_transcribe(&mut app, handle, model_path));
                                dirty = true;
                            }
                        }
                        KeyKind::Char('r') => {
                            if let Some(h) = voice_handle.take() {
                                h.cancel();
                            }
                            if voice_ptt_active {
                                tui.pop_voice_ptt();
                                voice_ptt_active = false;
                            }
                            match forge_voice::Recorder::start() {
                                Ok(new_handle) => {
                                    voice_ptt_active = tui.push_voice_ptt();
                                    voice_handle = Some(new_handle);
                                    voice_started_at = Some(Instant::now());
                                    app.voice =
                                        Some(forge_tui::VoiceOverlay::recording(voice_ptt_active));
                                    app.note("voice: restarted");
                                }
                                Err(e) => {
                                    voice_started_at = None;
                                    voice_model_path = None;
                                    app.voice = Some(forge_tui::VoiceOverlay::error(e.to_string()));
                                    voice_error_until =
                                        Some(Instant::now() + Duration::from_secs(2));
                                }
                            }
                            dirty = true;
                        }
                        _ => {}
                    },
                    forge_tui::VoicePhase::Transcribing => {
                        // Frozen: whisper runs off-thread via `spawn_blocking`, which has no
                        // cancel handle — swallow keys until the result (or failure) lands.
                    }
                }
                continue;
            }

            // The @path file-path picker is modal while open.
            if app.at_picker.open {
                match key {
                    KeyKind::Esc => app.at_picker.close(),
                    KeyKind::Up => app.at_picker.move_up(),
                    KeyKind::Down => app.at_picker.move_down(),
                    KeyKind::Tab | KeyKind::Enter => {
                        if let Some(path) = app.at_picker.selected_path() {
                            if let Some(tok) = forge_tui::at_token_at(
                                &app.input,
                                app.input_cursor.min(app.input.len()),
                            ) {
                                // Insert `@path ` (trailing space so the user can keep typing).
                                app.input
                                    .replace_range(tok.start..tok.end, &format!("@{path} "));
                                app.input_cursor = app.input.len();
                            } else {
                                app.input = format!("@{path} ");
                                app.input_cursor = app.input.len();
                            }
                        }
                        app.at_picker.close();
                    }
                    KeyKind::Char(c) => {
                        app.input.push(c);
                        sync_at_picker_to_at_token(&mut app);
                    }
                    KeyKind::Backspace => {
                        app.input.pop();
                        sync_at_picker_to_at_token(&mut app);
                    }
                    KeyKind::CycleTemper
                    | KeyKind::ToggleSubagentDetail
                    | KeyKind::ToggleEffortSlider
                    | KeyKind::SkipModel
                    | KeyKind::TierUp
                    | KeyKind::TierDown
                    | KeyKind::ToggleReasoning
                    | KeyKind::OpenKeybindConfig
                    | KeyKind::ModelPicker
                    | KeyKind::EffortCycle
                    | KeyKind::TemperCycle
                    | KeyKind::CopyLast
                    | KeyKind::ShowHelp
                    | KeyKind::SaveCheckpoint
                    | KeyKind::NewSession
                    | KeyKind::UndoWrite
                    | KeyKind::CompactSession
                    | KeyKind::ReloadConfig => {}
                    _ => {}
                }
                continue;
            }

            // The session/checkpoint picker is modal too: arrows navigate, typing filters, Enter
            // acts on the selection (resume / rewind), Esc cancels.
            if app.picker.open {
                match key {
                    KeyKind::Esc => {
                        // In the models browser, Esc from a drilled-in provider steps back to the
                        // provider list rather than closing the whole picker.
                        if app.picker.kind == Some(forge_tui::PickerKind::Models)
                            && app.models_drilled.is_some()
                        {
                            open_models_root(&session, &mut app).await?;
                        } else {
                            app.models_drilled = None;
                            app.models_pin_mode = false;
                            if app.picker.kind == Some(forge_tui::PickerKind::Duel) {
                                // Dropping `duel_state` drops every candidate's `WorktreeGuard`,
                                // removing its worktree dir + branch — no merge, no routing record.
                                duel_state = None;
                                app.note("⚔ duel discarded — no candidate was merged");
                            }
                            app.picker.close();
                        }
                    }
                    KeyKind::Up => app.picker.move_up(),
                    KeyKind::Down => app.picker.move_down(),
                    KeyKind::Tab if app.picker.kind == Some(forge_tui::PickerKind::Sessions) => {
                        let query = app.picker.query.clone();
                        app.show_archived = !app.show_archived;
                        open_sessions_picker(&mut app, &query)?;
                    }
                    KeyKind::DeleteForward
                        if app.picker.kind == Some(forge_tui::PickerKind::Sessions) =>
                    {
                        if let Some(row) = app.picker.selected_row() {
                            if !row.id.starts_with("observe:") {
                                let store = crate::open_store()?;
                                if app.show_archived {
                                    store.unarchive_session(&row.id)?;
                                } else {
                                    store.archive_session(&row.id)?;
                                }
                                let query = app.picker.query.clone();
                                open_sessions_picker(&mut app, &query)?;
                            }
                        }
                    }
                    KeyKind::Enter => {
                        let chosen = app.picker.selected_row().cloned();
                        let kind = app.picker.kind;
                        // The models browser drills (provider → models) on Enter instead of
                        // resolving; model rows are terminal. Keep the picker open either way.
                        // Exception: in pin-mode (bare `/model`) a leaf model row closes the picker
                        // and pins the selected model.
                        if kind == Some(forge_tui::PickerKind::Models) {
                            if let Some(row) = chosen {
                                if app.models_drilled.is_none() && !row.id.contains("::") {
                                    // Provider-level row → drill in.
                                    open_models_provider(&session, &mut app, &row.id).await?;
                                } else if row.id.contains("::") && app.models_pin_mode {
                                    // Leaf model row in pin-mode → pin it and close. `try_lock`
                                    // when available (idle), else fire-and-forget — same reasoning
                                    // as tier_up above; the picker is reachable mid-turn too.
                                    let model_id =
                                        forge_provider::normalize_model_id(&row.id).into_owned();
                                    if let Ok(mut s) = session.try_lock() {
                                        s.pin_model(Some(model_id.clone()));
                                    } else {
                                        let s = session.clone();
                                        let m = model_id.clone();
                                        tokio::spawn(
                                            async move { s.lock().await.pin_model(Some(m)) },
                                        );
                                    }
                                    app.models_pin_mode = false;
                                    app.models_drilled = None;
                                    app.picker.close();
                                    app.note(&format!(
                                        "⊕ model pinned: {model_id} (clears with /model)"
                                    ));
                                }
                            }
                            continue;
                        }
                        app.picker.close();
                        if let (Some(row), Some(kind)) = (chosen, kind) {
                            if kind == forge_tui::PickerKind::AssayChoice {
                                // Assay runs as a background task (like a turn) so the spinner
                                // ticks while critics + verification run.
                                turn_gen += 1;
                                let lenses = std::mem::take(&mut assay_lenses);
                                let scope = std::mem::replace(
                                    &mut assay_scope,
                                    forge_types::AssayScope::Repo,
                                );
                                turn_handle = spawn_assay(
                                    row.id == "cleanup",
                                    lenses,
                                    scope,
                                    &session,
                                    &done_tx,
                                    turn_gen,
                                    &mut app,
                                    &mut busy,
                                    &mut busy_since,
                                )
                                .await?;
                            } else if kind == forge_tui::PickerKind::CopyBlocks {
                                // Enter copies the selected candidate (full response or a block) to
                                // the clipboard. Row id is the index into copy_candidates.
                                if let Some((_, text)) = row
                                    .id
                                    .parse::<usize>()
                                    .ok()
                                    .and_then(|i| app.copy_candidates.get(i).cloned())
                                {
                                    let chars = text.chars().count();
                                    copy_selection(&mut clipboard, &text);
                                    app.note(&format!("✓ copied to clipboard ({chars} chars)"));
                                    // A remote client may have driven this picker — ship the
                                    // payload in the snapshot so the phone can copy it too.
                                    if remote.is_some() {
                                        remote_copy_text = Some(text);
                                    }
                                }
                                app.copy_candidates.clear();
                            } else if kind == forge_tui::PickerKind::Duel {
                                // Merge the picked branch back, record every candidate's outcome
                                // (won only for the winner) so future routing in this repo softly
                                // favors it, then drop every guard (removes every worktree+branch —
                                // the winner's diff is already applied, so its branch isn't needed).
                                if let Some((report, guards)) = duel_state.take() {
                                    let repo_root = std::env::current_dir()
                                        .unwrap_or_else(|_| std::path::PathBuf::from("."));
                                    let repo_key = repo_root.display().to_string();
                                    let winner_branch = row.id.clone();
                                    let merge_note = match forge_core::duel::merge_winner(
                                        &repo_root,
                                        &winner_branch,
                                    ) {
                                        Ok(m) if m.conflicted_files.is_empty() => {
                                            "merged cleanly".to_string()
                                        }
                                        Ok(m) => format!(
                                            "merged with conflicts in: {}",
                                            m.conflicted_files.join(", ")
                                        ),
                                        Err(e) => format!("merge failed: {e}"),
                                    };
                                    if let Ok(store) = crate::open_store() {
                                        for c in &report.candidates {
                                            let won = c.branch == winner_branch;
                                            let _ = store.record_duel_outcome(
                                                &repo_key,
                                                &c.model,
                                                won,
                                                &report.task,
                                            );
                                        }
                                    }
                                    let winner_model = report
                                        .candidates
                                        .iter()
                                        .find(|c| c.branch == winner_branch)
                                        .map(|c| c.model.clone())
                                        .unwrap_or_else(|| "?".to_string());
                                    app.note(&format!(
                                        "⚔ duel winner: {winner_model} — {merge_note}"
                                    ));
                                    drop(guards);
                                }
                            } else if kind == forge_tui::PickerKind::Sessions
                                && row.id.starts_with("observe:")
                            {
                                let session_id = row.id.trim_start_matches("observe:").to_string();
                                let obs_store = std::sync::Arc::new(crate::open_store()?);
                                let start_event_id =
                                    find_starting_event_id(&obs_store, &session_id);
                                // `try_lock`: same reasoning as skip_model above — a turn parked
                                // in a permission/question prompt holds this lock for the whole
                                // prompt, and this is the main render loop.
                                let Ok(mut s) = session.try_lock() else {
                                    app.note("⚠ try again in a moment — session is busy");
                                    dirty = true;
                                    continue;
                                };
                                s.reset_resumed(&session_id)
                                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                                let (items, view) = (s.replay_items_full(), s.view_snapshot());
                                drop(s);
                                tui.clear_screen();
                                app.clear_transcript();
                                app.note(&format!(
                                    "⚡ resumed live session {}",
                                    session_id.chars().take(8).collect::<String>()
                                ));
                                app.replay_history(&items);
                                if let Some(json) = view {
                                    app.restore_view_json(&json);
                                }
                                app.input =
                                    "⚡ Observing live MCP session — press Esc to stop".to_string();
                                observer = Some(ObserverState {
                                    session_id,
                                    store: obs_store,
                                    last_event_id: start_event_id,
                                    last_poll: std::time::Instant::now(),
                                });
                            } else {
                                picker_accept(kind, &row, &session, Some(&mut tui), &mut app)
                                    .await?;
                            }
                        }
                    }
                    // `w` in the copy picker writes the selected candidate to a file instead of the
                    // clipboard (useful over SSH). Other chars filter the list as usual.
                    KeyKind::Char(c)
                        if app.picker.kind == Some(forge_tui::PickerKind::CopyBlocks)
                            && (c == 'w' || c == 'W') =>
                    {
                        let pick = app
                            .picker
                            .selected_row()
                            .and_then(|r| r.id.parse::<usize>().ok())
                            .and_then(|i| app.copy_candidates.get(i).cloned());
                        app.picker.close();
                        app.copy_candidates.clear();
                        if let Some((lang, text)) = pick {
                            match write_copy_to_file(&text, &lang) {
                                Ok(path) => app.note(&format!("✓ wrote to {}", path.display())),
                                Err(e) => app.note(&format!("write failed: {e}")),
                            }
                        }
                    }
                    KeyKind::Char(c) => {
                        app.picker.query.push(c);
                        app.picker.clamp();
                    }
                    KeyKind::Backspace => {
                        app.picker.query.pop();
                        app.picker.clamp();
                    }
                    KeyKind::Tab
                    | KeyKind::CycleTemper
                    | KeyKind::ToggleSubagentDetail
                    | KeyKind::ToggleEffortSlider
                    | KeyKind::SkipModel
                    | KeyKind::TierUp
                    | KeyKind::TierDown
                    | KeyKind::ToggleReasoning
                    | KeyKind::OpenKeybindConfig
                    | KeyKind::ModelPicker
                    | KeyKind::EffortCycle
                    | KeyKind::TemperCycle
                    | KeyKind::CopyLast
                    | KeyKind::ShowHelp
                    | KeyKind::SaveCheckpoint
                    | KeyKind::NewSession
                    | KeyKind::UndoWrite
                    | KeyKind::CompactSession
                    | KeyKind::ReloadConfig => {}
                    _ => {}
                }
                continue;
            }

            // Full-screen mode: PageUp/PageDown scroll the transcript region. The render re-clamps
            // the offset to the visible area, so an over-scroll is harmless; here we approximate the
            // page (and the follow-resume threshold) from the terminal height.
            if app.fullscreen && matches!(key, KeyKind::PageUp | KeyKind::PageDown) {
                let body = tui.height().saturating_sub(8).max(1);
                if matches!(key, KeyKind::PageUp) {
                    app.transcript_scroll_up(body as usize);
                } else {
                    let (_, max_scroll) = app.transcript_metrics(tui.width(), body);
                    app.transcript_scroll_down(body as usize, max_scroll);
                }
                dirty = true;
                continue;
            }

            // Ctrl+End jumps the transcript to the tail and resumes following (mirrors clicking the
            // floating jump-to-bottom bar).
            if app.fullscreen && matches!(key, KeyKind::JumpBottom) {
                app.transcript_to_bottom();
                dirty = true;
                continue;
            }

            // Ctrl+O toggles focus on the sticky activity panel (main chat + subagents + critics).
            // When a workflow exists this turn (running or just finished) it opens the dedicated
            // workflow view instead — workflow rows live there, not in the activity panel.
            if matches!(key, KeyKind::ToggleSubagentDetail) {
                if app.workflow.exists() {
                    app.workflow.open = true;
                } else if app.has_activity() {
                    app.activity_focused = !app.activity_focused;
                    if app.activity_focused {
                        app.activity_idx =
                            app.activity_idx.min(app.activity_len().saturating_sub(1));
                    }
                }
                dirty = true;
                continue;
            }

            // While the activity panel has focus: ↑↓ move the selection (wrapping), Enter opens the
            // selected entry's full-screen transcript viewer, Esc unfocuses. Handled before the
            // global Esc so Esc steps out of the panel instead of quitting.
            if app.activity_focused {
                match key {
                    KeyKind::Up => {
                        let n = app.activity_len();
                        if n > 0 {
                            app.activity_idx = (app.activity_idx + n - 1) % n;
                        }
                    }
                    KeyKind::Down => {
                        let n = app.activity_len();
                        if n > 0 {
                            app.activity_idx = (app.activity_idx + 1) % n;
                        }
                    }
                    KeyKind::Enter => {
                        let idx = app.activity_idx;
                        if app.fullscreen {
                            // Full-screen: open the in-loop viewer (same terminal, no nested
                            // alt-screen). The main render loop keeps draining events, so the
                            // selected entry auto-updates while open.
                            app.open_viewer(idx);
                            app.activity_focused = false;
                        } else {
                            // Inline: the live region is tiny, so take over a separate alternate
                            // screen for the viewer and drain events in its refresh closure.
                            tui.run_fullscreen(|| {
                                forge_tui::run_transcript_viewer(idx, || {
                                    while let Ok(msg) = rx.try_recv() {
                                        match msg {
                                            UiMsg::Event(e) => app.apply(e),
                                            UiMsg::Permission { reply, .. } => {
                                                let _ = reply.send(ConfirmOutcome::Deny);
                                            }
                                            UiMsg::Question { reply, .. } => {
                                                let _ =
                                                    reply.send(forge_tui::NO_ANSWER.to_string());
                                            }
                                        }
                                    }
                                    app.activity_views()
                                })
                            })?;
                        }
                    }
                    KeyKind::Esc => {
                        app.activity_focused = false;
                    }
                    _ => {}
                }
                dirty = true;
                continue;
            }

            // Tab: accept the predicted next-prompt ghost text (see `render_input`) into an idle,
            // empty input — editable, never auto-sent. Every modal above already `continue`d, so
            // reaching here proves none is open; `take_suggestion_for_tab` only ever returns
            // `Some` for an empty input, and both busy/awaiting-question below clear the
            // suggestion on turn start, so it can never fire mid-turn.
            if matches!(key, KeyKind::Tab) {
                if let Some(text) = app.take_suggestion_for_tab() {
                    app.input = text;
                    app.input_cursor = app.input.len();
                    dirty = true;
                    continue;
                }
            }

            // Esc / Ctrl-C: while a turn is running it INTERRUPTS the AI (stops the response,
            // keeps Forge alive); while idle it quits. Checked before any prompt handling so the
            // user can never get wedged — interrupting also clears a pending permission/question.
            if matches!(key, KeyKind::Esc) {
                if busy {
                    if let Some(h) = turn_handle.take() {
                        h.abort(); // cancel the turn task; its DoneGuard drop releases the lock
                    }
                    turn_gen += 1; // discard the aborted turn's (now stale) done-signal
                    busy = false;
                    loop_state = None; // a `/loop` in progress stops on interrupt
                    goal_state = None; // a `/goal` in progress stops on interrupt
                    if !queued_prompts.is_empty() {
                        queued_prompts.clear(); // interrupting drops the queued prompts too
                        app.set_queued(&queued_prompts);
                    }
                    pending = None;
                    pending_question = None;
                    app.prompt = None;
                    app.clear_question();
                    // A live workflow run's WorkflowFinished will never arrive (its emitting
                    // task just died with the turn) — close it out as interrupted so the status
                    // band doesn't freeze and later turns don't inherit `active`.
                    app.workflow.on_interrupt();
                    app.apply(forge_tui::PresenterEvent::AssistantDone); // flush any partial reply
                    app.note("⏹ interrupted — stopped responding");
                    dirty = true;
                    continue;
                }
                quit = true;
                break;
            }
            if let Some((tool, reply)) = pending.take() {
                // Answering a permission prompt.
                let outcome = match key {
                    KeyKind::Char('a') | KeyKind::Char('A') => ConfirmOutcome::AlwaysAllow,
                    KeyKind::Char('y') | KeyKind::Char('Y') | KeyKind::Enter => {
                        ConfirmOutcome::Allow
                    }
                    _ => ConfirmOutcome::Deny,
                };
                let _ = reply.send(outcome);
                app.prompt = None;
                if outcome == ConfirmOutcome::AlwaysAllow {
                    if let Err(e) = forge_config::append_allow_rule(&tool) {
                        app.note(&format!("⚠ could not save allow rule: {e}"));
                    } else {
                        app.note(&format!("✓ {tool} added to .forge/config.toml allow rules"));
                    }
                }
            } else if app.awaiting_question() {
                // Answering an AskUserQuestion (the turn task is blocked in `ask()`): the input
                // line collects a number or free-text answer; submit resolves + replies.
                match handle_key(&mut app.input, &mut app.input_cursor, key) {
                    InputOutcome::Submit(line) => {
                        if let Some(ans) = app.resolve_question(&line) {
                            if let Some(tx) = pending_question.take() {
                                let _ = tx.send(ans);
                            }
                        } else {
                            app.input.clear(); // invalid → re-prompt (question stays open)
                        }
                    }
                    InputOutcome::Quit => {
                        quit = true;
                        break;
                    }
                    InputOutcome::Editing => {}
                }
            } else if busy {
                // Mid-turn: let the user keep typing and QUEUE submitted prompts to run after the
                // current turn finishes (Claude Code / aider style). Only plain text editing +
                // Enter is honored here; palette, commands, history and temper-cycling wait until
                // the turn is idle. A `/command` is held back (it needs the idle session).
                let outcome = if app.try_delete_paste_block(key) {
                    InputOutcome::Editing
                } else {
                    handle_key(&mut app.input, &mut app.input_cursor, key)
                };
                if let InputOutcome::Submit(raw_line) = outcome {
                    let (line, _imgs) = app.resolve_paste_blocks(raw_line);
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        // nothing to queue
                    } else if trimmed.starts_with('/') && !trimmed.starts_with("//") {
                        app.note("⏳ commands run when the turn is idle — finish or Esc first");
                    } else {
                        queued_prompts.push(line.clone());
                        app.set_queued(&queued_prompts);
                        app.note(&format!(
                            "⏳ queued ({} pending) — runs after this turn",
                            queued_prompts.len()
                        ));
                    }
                }
                dirty = true;
            } else if matches!(key, KeyKind::Char('f') | KeyKind::Char('F'))
                && app.pending_shell_fix.is_some()
            {
                // F: populate input with the pending shell fix command for the user to review.
                if let Some(fix) = app.pending_shell_fix.take() {
                    app.input = fix;
                }
            } else if matches!(key, KeyKind::CycleTemper) {
                // SHIFT+TAB: cycle the operating temper (idle only — never mid-turn).
                // `try_lock`: same reasoning as skip_model above; a no-op note on contention
                // rather than blocking the main render loop.
                let Some(new) = session.try_lock().ok().map(|mut sess| sess.cycle_temper()) else {
                    app.note("⚠ try again in a moment — session is busy");
                    dirty = true;
                    continue;
                };
                app.set_temper(new.label());
                // Remember the chosen temper as the default for the next session (best-effort).
                let _ = forge_config::write_permission_mode(new);
            } else if matches!(key, KeyKind::TemperCycle) {
                // Alt-T: same as SHIFT+TAB temper cycling but via configurable keybind.
                // `try_lock`: same reasoning as skip_model above; a no-op note on contention
                // rather than blocking the main render loop.
                let Some(new) = session.try_lock().ok().map(|mut sess| sess.cycle_temper()) else {
                    app.note("⚠ try again in a moment — session is busy");
                    dirty = true;
                    continue;
                };
                app.set_temper(new.label());
                let _ = forge_config::write_permission_mode(new);
            } else if matches!(key, KeyKind::ModelPicker) {
                // Alt-M: open the model browser in pin-mode.
                app.models_pin_mode = true;
                open_models_root(&session, &mut app).await?;
            } else if matches!(key, KeyKind::NewSession) {
                // Ctrl-N: start a fresh session (equivalent to /new).
                if let DispatchOutcome::Quit = dispatch_command(
                    "/new",
                    &session,
                    Some(&mut tui),
                    &mut app,
                    &catalog,
                    &mut armed_project,
                    trust_project,
                    busy,
                    &mut assay_lenses,
                    &mut assay_scope,
                )
                .await?
                {
                    abort_turn_before_quit(
                        &mut turn_handle,
                        &mut pending,
                        &mut pending_question,
                        &mut app,
                    );
                    quit = true;
                    break;
                }
            } else if matches!(key, KeyKind::SaveCheckpoint) {
                // Ctrl-S: save a checkpoint (equivalent to /checkpoint).
                if let DispatchOutcome::Quit = dispatch_command(
                    "/checkpoint",
                    &session,
                    Some(&mut tui),
                    &mut app,
                    &catalog,
                    &mut armed_project,
                    trust_project,
                    busy,
                    &mut assay_lenses,
                    &mut assay_scope,
                )
                .await?
                {
                    abort_turn_before_quit(
                        &mut turn_handle,
                        &mut pending,
                        &mut pending_question,
                        &mut app,
                    );
                    quit = true;
                    break;
                }
            } else if matches!(key, KeyKind::UndoWrite) {
                // Ctrl-Z: undo last write (equivalent to /undo).
                if let DispatchOutcome::Quit = dispatch_command(
                    "/undo",
                    &session,
                    Some(&mut tui),
                    &mut app,
                    &catalog,
                    &mut armed_project,
                    trust_project,
                    busy,
                    &mut assay_lenses,
                    &mut assay_scope,
                )
                .await?
                {
                    abort_turn_before_quit(
                        &mut turn_handle,
                        &mut pending,
                        &mut pending_question,
                        &mut app,
                    );
                    quit = true;
                    break;
                }
            } else if matches!(key, KeyKind::CompactSession) {
                // Ctrl-L: compact the conversation (equivalent to /compact).
                match dispatch_command(
                    "/compact",
                    &session,
                    Some(&mut tui),
                    &mut app,
                    &catalog,
                    &mut armed_project,
                    trust_project,
                    busy,
                    &mut assay_lenses,
                    &mut assay_scope,
                )
                .await?
                {
                    DispatchOutcome::RunCompact => {
                        turn_gen += 1;
                        turn_handle = Some(spawn_compact(
                            &session,
                            &done_tx,
                            turn_gen,
                            &mut app,
                            &mut busy,
                            &mut busy_since,
                        ));
                    }
                    DispatchOutcome::Quit => {
                        abort_turn_before_quit(
                            &mut turn_handle,
                            &mut pending,
                            &mut pending_question,
                            &mut app,
                        );
                        quit = true;
                        break;
                    }
                    _ => {}
                }
            } else if let (true, Some(up)) = (
                matches!(key, KeyKind::Up),
                forge_tui::input_cursor_up(&app.input, app.input_cursor),
            ) {
                // Multiline draft, cursor below the first row: move the cursor up a line instead of
                // clobbering the draft with history recall (history only fires from the first row).
                app.input_cursor = up;
                dirty = true;
            } else if matches!(key, KeyKind::Up) {
                // Arrow-up on the first row (or single-line): browse the previous prompt history.
                if history_pos.is_none() {
                    history_draft = app.input.clone();
                }
                if let Some(p) = history_pos {
                    if p > 0 {
                        history_pos = Some(p - 1);
                    }
                } else if !prompt_history.is_empty() {
                    history_pos = Some(prompt_history.len() - 1);
                }
                if let Some(p) = history_pos {
                    app.input = prompt_history[p].clone();
                    app.input_cursor = app.input.len();
                }
                dirty = true;
            } else if matches!(key, KeyKind::Down) {
                // Arrow-down: browse to the next entry, or restore the draft past the end.
                if let Some(p) = history_pos {
                    if p + 1 < prompt_history.len() {
                        history_pos = Some(p + 1);
                        app.input = prompt_history[p + 1].clone();
                        app.input_cursor = app.input.len();
                    } else {
                        history_pos = None;
                        app.input = history_draft.clone();
                        app.input_cursor = app.input.len();
                    }
                }
                dirty = true;
            } else {
                let pre_edit_len = app.input.len();
                let outcome = if app.try_delete_paste_block(key) {
                    InputOutcome::Editing
                } else {
                    handle_key(&mut app.input, &mut app.input_cursor, key)
                };
                match outcome {
                    InputOutcome::Submit(raw_line) => {
                        let (line, submit_images) = app.resolve_paste_blocks(raw_line);
                        history_pos = None;
                        if !line.trim().is_empty() && prompt_history.last() != Some(&line) {
                            prompt_history.push(line.clone());
                        }
                        // `//foo` escapes to a literal prompt `/foo`; a bare `/cmd` typed without
                        // the palette still dispatches as a command; everything else is a prompt.
                        if let Some(rest) = line.strip_prefix("//") {
                            let hooks = session.lock().await.hooks().to_vec();
                            let escaped = format!("/{rest}");
                            match forge_core::hooks::run_prompt_hooks(&hooks, &escaped).await {
                                Err(reason) => {
                                    app.note(&format!("⎇ prompt blocked by hook: {reason}"));
                                }
                                Ok(prompt) => {
                                    turn_gen += 1;
                                    turn_handle = Some(spawn_turn(
                                        &prompt,
                                        &session,
                                        &done_tx,
                                        turn_gen,
                                        &mut app,
                                        &mut busy,
                                        &mut busy_since,
                                    ));
                                }
                            }
                        } else if line.starts_with('/') {
                            match dispatch_command(
                                &line,
                                &session,
                                Some(&mut tui),
                                &mut app,
                                &catalog,
                                &mut armed_project,
                                trust_project,
                                busy,
                                &mut assay_lenses,
                                &mut assay_scope,
                            )
                            .await?
                            {
                                DispatchOutcome::Quit => {
                                    abort_turn_before_quit(
                                        &mut turn_handle,
                                        &mut pending,
                                        &mut pending_question,
                                        &mut app,
                                    );
                                    quit = true;
                                    break;
                                }
                                DispatchOutcome::Handled => {}
                                DispatchOutcome::RunTurn {
                                    prompt,
                                    guidance,
                                    tier,
                                } => {
                                    turn_gen += 1;
                                    turn_handle = Some(spawn_turn_with(
                                        prompt,
                                        guidance,
                                        tier,
                                        &session,
                                        &done_tx,
                                        turn_gen,
                                        &mut app,
                                        &mut busy,
                                        &mut busy_since,
                                    ));
                                }
                                DispatchOutcome::RunCompact => {
                                    turn_gen += 1;
                                    turn_handle = Some(spawn_compact(
                                        &session,
                                        &done_tx,
                                        turn_gen,
                                        &mut app,
                                        &mut busy,
                                        &mut busy_since,
                                    ));
                                }
                                DispatchOutcome::RunSavedWorkflow { name, args } => {
                                    turn_gen += 1;
                                    turn_handle = Some(spawn_saved_workflow(
                                        &session,
                                        &done_tx,
                                        turn_gen,
                                        &mut app,
                                        &mut busy,
                                        &mut busy_since,
                                        name,
                                        args,
                                    ));
                                }
                                DispatchOutcome::RunDuel { task } => {
                                    turn_gen += 1;
                                    turn_handle = Some(spawn_duel(
                                        &session,
                                        &done_tx,
                                        turn_gen,
                                        &mut app,
                                        &mut busy,
                                        &mut busy_since,
                                        task,
                                        Arc::clone(&pending_duel),
                                    ));
                                }
                                DispatchOutcome::StartLoop { prompt } => {
                                    turn_gen += 1;
                                    loop_state = Some(LoopState {
                                        gen: turn_gen,
                                        iter: 1,
                                    });
                                    app.note("↻ loop started — Esc to stop");
                                    turn_handle = Some(spawn_turn_with(
                                        prompt,
                                        vec![LOOP_GUIDANCE.to_string()],
                                        None,
                                        &session,
                                        &done_tx,
                                        turn_gen,
                                        &mut app,
                                        &mut busy,
                                        &mut busy_since,
                                    ));
                                }
                                DispatchOutcome::StartGoal { prompt, goal } => {
                                    turn_gen += 1;
                                    goal_state = Some(GoalState {
                                        gen: turn_gen,
                                        iter: 1,
                                        prev_done: 0,
                                        no_progress: 0,
                                        goal,
                                    });
                                    app.note("🎯 goal started — Esc to stop");
                                    turn_handle = Some(spawn_turn_with(
                                        prompt,
                                        vec![GOAL_GUIDANCE.to_string()],
                                        Some(forge_types::TaskTier::Complex),
                                        &session,
                                        &done_tx,
                                        turn_gen,
                                        &mut app,
                                        &mut busy,
                                        &mut busy_since,
                                    ));
                                }
                                DispatchOutcome::PendingMesh(rx) => {
                                    mesh_load_rx = Some(rx);
                                }
                                DispatchOutcome::PendingUsage(rx) => {
                                    usage_load_rx = Some(rx);
                                }
                                DispatchOutcome::PendingVoice(start) => {
                                    apply_voice_start(
                                        start,
                                        &mut tui,
                                        &mut app,
                                        &mut voice_handle,
                                        &mut voice_model_path,
                                        &mut voice_download_progress_rx,
                                        &mut voice_download_done_rx,
                                        &mut voice_started_at,
                                        &mut voice_error_until,
                                        &mut voice_ptt_active,
                                    );
                                }
                                DispatchOutcome::ToggleRemote { exposure } => {
                                    toggle_remote(
                                        &mut remote,
                                        &mut app,
                                        &mut tui,
                                        exposure,
                                        &tui_config.remote,
                                        remote_history.clone(),
                                    )
                                    .await?;
                                    // Fresh server, fresh watch channel — reset the change-only
                                    // broadcast dedup so the first frame always goes out.
                                    last_remote_snap = None;
                                }
                                DispatchOutcome::CopyToClipboard(text) => {
                                    let chars = text.chars().count();
                                    copy_selection(&mut clipboard, &text);
                                    app.note(&format!(
                                        "✓ copied response to clipboard ({chars} chars)"
                                    ));
                                    if remote.is_some() {
                                        remote_copy_text = Some(text);
                                    }
                                }
                            }
                        } else {
                            let hooks = session.lock().await.hooks().to_vec();
                            match forge_core::hooks::run_prompt_hooks(&hooks, &line).await {
                                Err(reason) => {
                                    app.note(&format!("⎇ prompt blocked by hook: {reason}"));
                                }
                                Ok(prompt) => {
                                    // Attach any images pasted/added into this prompt as vision
                                    // input for the turn about to run.
                                    if !submit_images.is_empty() {
                                        session.lock().await.attach_images(submit_images);
                                    }
                                    // Expand `@path` mentions: read those files and ride their
                                    // contents along as turn guidance, leaving the echoed line clean.
                                    let (file_blocks, included, skipped) = expand_at_files(&prompt);
                                    if !included.is_empty() {
                                        app.note(&format!("📎 included {}", included.join(", ")));
                                    }
                                    for s in &skipped {
                                        app.note(&format!("⚠ skipped {s}"));
                                    }
                                    last_prompt = Some(prompt.clone());
                                    // A genuinely new prompt — any Ctrl+K skips from a PREVIOUS
                                    // request no longer apply to this one.
                                    skip_model_excludes.clear();
                                    turn_gen += 1;
                                    turn_handle = Some(if file_blocks.is_empty() {
                                        spawn_turn(
                                            &prompt,
                                            &session,
                                            &done_tx,
                                            turn_gen,
                                            &mut app,
                                            &mut busy,
                                            &mut busy_since,
                                        )
                                    } else {
                                        spawn_turn_with(
                                            prompt.clone(),
                                            file_blocks,
                                            None,
                                            &session,
                                            &done_tx,
                                            turn_gen,
                                            &mut app,
                                            &mut busy,
                                            &mut busy_since,
                                        )
                                    });
                                }
                            }
                        }
                    }
                    InputOutcome::Quit => {
                        quit = true;
                        break;
                    }
                    InputOutcome::Editing => {
                        if app.input.len() != pre_edit_len {
                            history_pos = None;
                        }
                        // `/command` anywhere on the line opens the palette; `@path` opens the
                        // file picker. They are mutually exclusive — slash wins at cursor.
                        // Cursor-anchored, same as `sync_palette_to_slash_token`: `slash_token_at`
                        // falls back to the LAST slash token on the line when the cursor isn't
                        // inside any token, so without this filter, typing args past a command
                        // name (cursor now beyond the token) re-opened the palette on every
                        // keystroke here — immediately closed again next keystroke by the OTHER
                        // (correctly filtered) palette-open branch, producing a flash/reopen loop.
                        let cur = app.input_cursor.min(app.input.len());
                        let tok = forge_tui::slash_token_at(&app.input, cur)
                            .filter(|t| cur >= t.start && cur <= t.end);
                        if let Some(tok) = tok {
                            app.at_picker.close();
                            app.palette.open_with(&tok.name);
                        } else {
                            app.palette.close();
                            sync_at_picker_to_at_token(&mut app);
                        }
                    }
                }
            }
        }
        if quit {
            break;
        }

        while let Ok(msg) = rx.try_recv() {
            dirty = true;
            match msg {
                UiMsg::Event(e) => app.apply(e),
                UiMsg::Permission {
                    tool,
                    side_effect,
                    reply,
                } => {
                    app.prompt = Some(format!("allow {tool} ({side_effect:?}) [y/n/a=always]"));
                    pending = Some((tool, reply));
                    // New prompt, new identity: remote answers rendered against an older prompt
                    // now carry a stale seq and are ignored instead of approving this one.
                    prompt_seq += 1;
                }
                UiMsg::Question {
                    question,
                    options,
                    allow_other,
                    reply,
                } => {
                    app.set_question(&question, &options, allow_other);
                    pending_question = Some(reply);
                    prompt_seq += 1;
                }
            }
        }

        // Keep the commit hook's model file current with whichever model ran the latest turn, so a
        // commit the agent makes is attributed to the model that actually did the work.
        if git_coauthor {
            if let Some(model) = app.routing.as_ref().map(|r| r.model.clone()) {
                if !model.is_empty() && model != last_model_written {
                    write_active_model(&model);
                    last_model_written = model;
                }
            }
        }

        // Drain remote-control inputs (a browser sent a prompt / answer / interrupt) and inject
        // them exactly like local keystrokes. We process the whole queue each iteration so a
        // chatty phone can't fall behind. Each input marks `dirty` (the statusline/preview may
        // change) and may spawn a turn / answer a prompt.
        if let Some(rc) = remote.as_mut() {
            while let Ok(input) = rc.input_rx.try_recv() {
                dirty = true;
                match input {
                    remote::RemoteInput::Prompt { text, attachments } => {
                        // A fresh prompt starts a fresh interaction — drop the previous notices
                        // (and the served /copy payload) so they don't linger on the page forever.
                        remote_notes.clear();
                        remote_copy_text = None;

                        // A message-correlated attachment list (mobile upload race fix) is
                        // authoritative for THIS turn when non-empty: any stale ambient upload
                        // for an unrelated adjacent message must not leak in, so it's discarded
                        // up front — image attachment happens here too (it already applied
                        // ambiently regardless of dispatch branch before this fix, so this keeps
                        // that same reach); non-image mentions are resolved into
                        // `explicit_mentions` but only actually prepended onto `text` in the
                        // plain-prompt branch below, exactly where the old ambient
                        // `remote_attach_mentions` were — so a `//`-escape or `/command` typed
                        // alongside an attachment still parses the ORIGINAL text, unchanged.
                        let has_explicit_attachments = !attachments.is_empty();
                        if has_explicit_attachments {
                            remote_attach_mentions.clear();
                        }
                        let mut explicit_mentions = resolve_prompt_attachments(
                            &session,
                            &mut app,
                            &mut remote_notes,
                            &remote_cwd,
                            attachments,
                        )
                        .await;
                        // `/keys` is host-only BY DESIGN: it runs a blocking fullscreen
                        // configurator loop on the host terminal — dispatching it from here
                        // would freeze the host TUI on a screen the remote can't see or drive.
                        if !busy
                            && matches!(
                                forge_tui::parse_command(&text),
                                forge_tui::CommandAction::Keys
                            )
                        {
                            push_remote_note(
                                &mut remote_notes,
                                "⌨ /keys is host-only (a fullscreen keybind configurator on the host terminal)",
                            );
                            continue;
                        }
                        if busy {
                            // Mid-turn: queue plain prompts to run after this turn, exactly like
                            // local typing (they used to be refused here, so the phone couldn't
                            // do what the keyboard could). `/commands` still wait for idle —
                            // they need the session lock the running turn holds.
                            let trimmed = text.trim();
                            if trimmed.is_empty() {
                                // nothing to queue
                            } else if trimmed.starts_with('/') && !trimmed.starts_with("//") {
                                app.note(
                                    "⏳ commands run when the turn is idle — finish or Esc first",
                                );
                            } else {
                                queued_prompts.push(text.clone());
                                app.set_queued(&queued_prompts);
                                app.note(&format!(
                                    "⏳ queued ({} pending) — runs after this turn",
                                    queued_prompts.len()
                                ));
                            }
                        } else if let Some(rest) = text.strip_prefix("//") {
                            let hooks = session.lock().await.hooks().to_vec();
                            let escaped = format!("/{rest}");
                            if let Ok(prompt) =
                                forge_core::hooks::run_prompt_hooks(&hooks, &escaped).await
                            {
                                turn_gen += 1;
                                turn_handle = Some(spawn_turn(
                                    &prompt,
                                    &session,
                                    &done_tx,
                                    turn_gen,
                                    &mut app,
                                    &mut busy,
                                    &mut busy_since,
                                ));
                            }
                        } else if text.starts_with('/') {
                            match dispatch_command(
                                &text,
                                &session,
                                Some(&mut tui),
                                &mut app,
                                &catalog,
                                &mut armed_project,
                                trust_project,
                                busy,
                                &mut assay_lenses,
                                &mut assay_scope,
                            )
                            .await?
                            {
                                DispatchOutcome::Quit => {
                                    abort_turn_before_quit(
                                        &mut turn_handle,
                                        &mut pending,
                                        &mut pending_question,
                                        &mut app,
                                    );
                                    quit = true;
                                    break;
                                }
                                DispatchOutcome::RunTurn {
                                    prompt,
                                    guidance,
                                    tier,
                                } => {
                                    turn_gen += 1;
                                    turn_handle = Some(spawn_turn_with(
                                        prompt,
                                        guidance,
                                        tier,
                                        &session,
                                        &done_tx,
                                        turn_gen,
                                        &mut app,
                                        &mut busy,
                                        &mut busy_since,
                                    ));
                                }
                                DispatchOutcome::RunCompact => {
                                    turn_gen += 1;
                                    turn_handle = Some(spawn_compact(
                                        &session,
                                        &done_tx,
                                        turn_gen,
                                        &mut app,
                                        &mut busy,
                                        &mut busy_since,
                                    ));
                                }
                                DispatchOutcome::RunSavedWorkflow { name, args } => {
                                    turn_gen += 1;
                                    turn_handle = Some(spawn_saved_workflow(
                                        &session,
                                        &done_tx,
                                        turn_gen,
                                        &mut app,
                                        &mut busy,
                                        &mut busy_since,
                                        name,
                                        args,
                                    ));
                                }
                                DispatchOutcome::RunDuel { task } => {
                                    turn_gen += 1;
                                    turn_handle = Some(spawn_duel(
                                        &session,
                                        &done_tx,
                                        turn_gen,
                                        &mut app,
                                        &mut busy,
                                        &mut busy_since,
                                        task,
                                        Arc::clone(&pending_duel),
                                    ));
                                }
                                DispatchOutcome::StartLoop { prompt } => {
                                    turn_gen += 1;
                                    loop_state = Some(LoopState {
                                        gen: turn_gen,
                                        iter: 1,
                                    });
                                    app.note("↻ loop started — Esc to stop");
                                    turn_handle = Some(spawn_turn_with(
                                        prompt,
                                        vec![LOOP_GUIDANCE.to_string()],
                                        None,
                                        &session,
                                        &done_tx,
                                        turn_gen,
                                        &mut app,
                                        &mut busy,
                                        &mut busy_since,
                                    ));
                                }
                                DispatchOutcome::StartGoal { prompt, goal } => {
                                    turn_gen += 1;
                                    goal_state = Some(GoalState {
                                        gen: turn_gen,
                                        iter: 1,
                                        prev_done: 0,
                                        no_progress: 0,
                                        goal,
                                    });
                                    app.note("🎯 goal started — Esc to stop");
                                    turn_handle = Some(spawn_turn_with(
                                        prompt,
                                        vec![GOAL_GUIDANCE.to_string()],
                                        Some(forge_types::TaskTier::Complex),
                                        &session,
                                        &done_tx,
                                        turn_gen,
                                        &mut app,
                                        &mut busy,
                                        &mut busy_since,
                                    ));
                                }
                                DispatchOutcome::Handled => {}
                                DispatchOutcome::PendingMesh(rx) => {
                                    // The overlay opens loading=true and is projected into
                                    // `Snapshot::overlay`, so the phone sees it live.
                                    mesh_load_rx = Some(rx);
                                }
                                DispatchOutcome::PendingUsage(rx) => {
                                    usage_load_rx = Some(rx);
                                }
                                DispatchOutcome::PendingVoice(start) => {
                                    // Recording happens on the HOST (where the mic actually is)
                                    // regardless of which device issued `/voice` — same as every
                                    // other remotely-triggered command.
                                    apply_voice_start(
                                        start,
                                        &mut tui,
                                        &mut app,
                                        &mut voice_handle,
                                        &mut voice_model_path,
                                        &mut voice_download_progress_rx,
                                        &mut voice_download_done_rx,
                                        &mut voice_started_at,
                                        &mut voice_error_until,
                                        &mut voice_ptt_active,
                                    );
                                }
                                DispatchOutcome::ToggleRemote { .. } => {
                                    // Can't be honored here: this drain runs under the
                                    // `remote.as_mut()` borrow that toggling would destroy.
                                    // Silence looked like a swallowed command — say so instead.
                                    push_remote_note(
                                        &mut remote_notes,
                                        "⚠ /remote can only be toggled from the TUI",
                                    );
                                }
                                DispatchOutcome::CopyToClipboard(text) => {
                                    let chars = text.chars().count();
                                    copy_selection(&mut clipboard, &text);
                                    push_remote_note(
                                        &mut remote_notes,
                                        &format!(
                                            "✓ copied on the host ({chars} chars) — tap “Copy here” below for this device"
                                        ),
                                    );
                                    // Ship the payload in the snapshot so the PHONE can copy it.
                                    remote_copy_text = Some(text);
                                }
                            }
                        } else {
                            // Uploaded text files ride this prompt as @path mentions — the
                            // explicit, message-correlated list (if this prompt carried one) is
                            // authoritative; otherwise fall back to exactly the old ambient
                            // `Attach`-then-`Prompt` behavior.
                            let text = if has_explicit_attachments {
                                prepend_attach_mentions(&mut explicit_mentions, text)
                            } else {
                                prepend_attach_mentions(&mut remote_attach_mentions, text)
                            };
                            let hooks = session.lock().await.hooks().to_vec();
                            if let Ok(prompt) =
                                forge_core::hooks::run_prompt_hooks(&hooks, &text).await
                            {
                                // Expand `@path` mentions exactly like the local submit path
                                // (the remote prompt used to skip this — uploads made the gap
                                // load-bearing).
                                let (file_blocks, included, skipped) = expand_at_files(&prompt);
                                if !included.is_empty() {
                                    app.note(&format!("📎 included {}", included.join(", ")));
                                }
                                for s in &skipped {
                                    app.note(&format!("⚠ skipped {s}"));
                                }
                                turn_gen += 1;
                                turn_handle = Some(if file_blocks.is_empty() {
                                    spawn_turn(
                                        &prompt,
                                        &session,
                                        &done_tx,
                                        turn_gen,
                                        &mut app,
                                        &mut busy,
                                        &mut busy_since,
                                    )
                                } else {
                                    spawn_turn_with(
                                        prompt.clone(),
                                        file_blocks,
                                        None,
                                        &session,
                                        &done_tx,
                                        turn_gen,
                                        &mut app,
                                        &mut busy,
                                        &mut busy_since,
                                    )
                                });
                            }
                        }
                    }
                    remote::RemoteInput::Attach { path, image } => {
                        handle_remote_attach(
                            &session,
                            &mut app,
                            &mut remote_attach_mentions,
                            &remote_cwd,
                            path,
                            image,
                        )
                        .await;
                    }
                    remote::RemoteInput::Allow { yes, seq } => {
                        // The seq must target the prompt pending NOW: a new prompt can be
                        // installed in the SAME loop iteration (UiMsg drain runs just above),
                        // so an un-gated Allow could approve a newer, more dangerous prompt
                        // the operator never saw.
                        if !remote::prompt_seq_current(prompt_seq, seq) {
                            push_remote_note(
                                &mut remote_notes,
                                "⚠ stale answer ignored — the prompt changed; review the current one",
                            );
                        } else if let Some((tool, reply)) = pending.take() {
                            let outcome = if yes {
                                ConfirmOutcome::Allow
                            } else {
                                ConfirmOutcome::Deny
                            };
                            let _ = reply.send(outcome);
                            app.prompt = None;
                            if yes {
                                app.note(&format!("✓ remote allowed {tool}"));
                            } else {
                                app.note(&format!("✗ remote denied {tool}"));
                            }
                        }
                    }
                    remote::RemoteInput::Answer { text, seq } => {
                        if !remote::prompt_seq_current(prompt_seq, seq) {
                            push_remote_note(
                                &mut remote_notes,
                                "⚠ stale answer ignored — the prompt changed; review the current one",
                            );
                        } else if app.awaiting_question() {
                            if let Some(ans) = app.resolve_question(&text) {
                                if let Some(tx) = pending_question.take() {
                                    let _ = tx.send(ans);
                                }
                            } else {
                                app.note("⚠ remote answer was invalid — re-asking");
                            }
                        }
                    }
                    remote::RemoteInput::Interrupt => {
                        if busy {
                            if let Some(h) = turn_handle.take() {
                                h.abort();
                            }
                            turn_gen += 1;
                            busy = false;
                            loop_state = None;
                            goal_state = None;
                            pending = None;
                            pending_question = None;
                            app.prompt = None;
                            app.clear_question();
                            app.apply(forge_tui::PresenterEvent::AssistantDone);
                            app.note("⏹ remote interrupted — stopped responding");
                        }
                    }
                    remote::RemoteInput::Dequeue { index, text } => {
                        let idx = index as usize;
                        if idx < queued_prompts.len() && queued_prompts[idx] == text {
                            queued_prompts.remove(idx);
                            app.set_queued(&queued_prompts);
                            app.note(&format!(
                                "✕ remote dequeued — {} pending",
                                queued_prompts.len()
                            ));
                        } else {
                            push_remote_note(
                                &mut remote_notes,
                                "⚠ stale dequeue ignored — the queue changed; review the current list",
                            );
                        }
                    }
                    remote::RemoteInput::Key { key } => {
                        // The keystroke channel: inject through the SAME key path a local
                        // keystroke takes (queued here, drained at the head of the key loop).
                        // Two guards, both security/UX — never parity — motivated:
                        // 1. While a permission prompt / question is pending, raw keys could
                        //    resolve it WITHOUT the prompt_seq check that protects taps from
                        //    approving a newer prompt they never saw — those must go through
                        //    the seq-checked Allow/Answer inputs only.
                        // 2. Esc with nothing modal open and no turn running QUITS the host
                        //    TUI locally; remotely that's an accidental one-tap host kill.
                        if pending.is_some() || app.awaiting_question() {
                            push_remote_note(
                                &mut remote_notes,
                                "⚠ a prompt is pending — answer it with its buttons",
                            );
                        } else {
                            match remote::named_key(&key) {
                                Some(forge_tui::KeyKind::Esc)
                                    if !busy && !any_remote_modal_open(&app) =>
                                {
                                    push_remote_note(
                                        &mut remote_notes,
                                        "Esc ignored — nothing to close (use /quit to exit the host)",
                                    );
                                }
                                Some(k) => remote_keys.push_back(k),
                                None => push_remote_note(
                                    &mut remote_notes,
                                    &format!("⚠ unknown key {key:?} ignored"),
                                ),
                            }
                        }
                    }
                    remote::RemoteInput::OverlaySelect { id } => {
                        remote_keys
                            .extend(apply_overlay_input(&mut app, RemoteOverlayOp::Select(id)));
                    }
                    remote::RemoteInput::OverlayNav { delta } => {
                        remote_keys
                            .extend(apply_overlay_input(&mut app, RemoteOverlayOp::Nav(delta)));
                    }
                    remote::RemoteInput::OverlayFilter { text } => {
                        remote_keys
                            .extend(apply_overlay_input(&mut app, RemoteOverlayOp::Filter(text)));
                    }
                    remote::RemoteInput::OverlayCancel => {
                        remote_keys.extend(apply_overlay_input(&mut app, RemoteOverlayOp::Cancel));
                    }
                }
            }
        }
        if quit {
            break;
        }

        // Clear busy only on the *current* turn's done-signal; a stale signal from an interrupted
        // (aborted) turn carries an older generation and is ignored.
        while let Ok(g) = done_rx.try_recv() {
            if busy && g == turn_gen {
                busy = false;
                turn_handle = None;
                dirty = true;
                // Persist the on-screen view (activity panel, viewer, scroll) as of this completed
                // turn so a later resume restores it exactly. Skipped when there's nothing to save.
                if let Some(json) = app.view_snapshot_json() {
                    session.lock().await.save_view_snapshot(&json);
                }
                // `/duel`: a finished report is waiting in `pending_duel` — open the picker over
                // its candidates (diffstat / test badge / duration / cost) and hold the report +
                // worktree guards in `duel_state` until the user picks a winner or cancels.
                if let Some((report, guards)) = pending_duel.lock().unwrap().take() {
                    if report.candidates.is_empty() {
                        app.note("⚔ duel produced no usable candidates");
                    } else {
                        let rows = duel_picker_rows(&report);
                        app.picker.open_with(
                            forge_tui::PickerKind::Duel,
                            &format!("⚔ duel — pick the winner ({} candidates)", rows.len()),
                            rows,
                        );
                        duel_state = Some((report, guards));
                    }
                }
                // `/loop`: if this was a loop turn, decide whether to run another iteration.
                if let Some(ls) = loop_state.take() {
                    if ls.gen == g {
                        let last = {
                            session
                                .lock()
                                .await
                                .last_assistant_text()
                                .map(str::to_string)
                        };
                        match loop_stop_reason(last.as_deref(), ls.iter) {
                            Some(reason) => app.note(reason),
                            None => {
                                turn_gen += 1;
                                loop_state = Some(LoopState {
                                    gen: turn_gen,
                                    iter: ls.iter + 1,
                                });
                                turn_handle = Some(spawn_turn_with(
                                    "Continue toward completion.".to_string(),
                                    vec![LOOP_GUIDANCE.to_string()],
                                    None,
                                    &session,
                                    &done_tx,
                                    turn_gen,
                                    &mut app,
                                    &mut busy,
                                    &mut busy_since,
                                ));
                            }
                        }
                    } else {
                        loop_state = Some(ls); // a different turn finished; keep waiting
                    }
                }
                // `/goal`: if this was a goal turn, decide whether to run another iteration off
                // the tracked task plan (a session is either looping or goaling, never both).
                if let Some(gs) = goal_state.take() {
                    if gs.gen == g {
                        let (done, total) = {
                            let s = session.lock().await;
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
                            session
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
                            Some(reason) => app.note(reason),
                            None => {
                                turn_gen += 1;
                                goal_state = Some(GoalState {
                                    gen: turn_gen,
                                    iter: gs.iter + 1,
                                    prev_done: done,
                                    no_progress,
                                    goal: gs.goal,
                                });
                                turn_handle = Some(spawn_turn_with(
                                    GOAL_CONTINUE_PROMPT.to_string(),
                                    vec![GOAL_GUIDANCE.to_string()],
                                    Some(forge_types::TaskTier::Complex),
                                    &session,
                                    &done_tx,
                                    turn_gen,
                                    &mut app,
                                    &mut busy,
                                    &mut busy_since,
                                ));
                            }
                        }
                    } else {
                        goal_state = Some(gs); // a different turn finished; keep waiting
                    }
                }
                // Drain a queued prompt (typed while this turn was running): run it as the next
                // turn, ahead of auto-compaction (the queued turn auto-compacts itself if needed).
                if turn_handle.is_none() && !queued_prompts.is_empty() {
                    let next = queued_prompts.remove(0);
                    app.set_queued(&queued_prompts);
                    if prompt_history.last() != Some(&next) {
                        prompt_history.push(next.clone());
                    }
                    turn_gen += 1;
                    turn_handle = Some(spawn_turn(
                        &next,
                        &session,
                        &done_tx,
                        turn_gen,
                        &mut app,
                        &mut busy,
                        &mut busy_since,
                    ));
                }
                // Auto-compact: when no new turn was spawned (not a loop iteration) and the
                // context gauge is above AUTO_COMPACT_THRESHOLD, quietly run /compact so the
                // user doesn't need to do it manually (context-compaction.md).
                // Guard: only fire once per user turn — compact's own Cost event still carries
                // the old full-context size, so context_tokens won't drop until the next real
                // turn. Without the gen guard this would re-fire on every compact completion.
                if turn_handle.is_none() && turn_gen > last_auto_compact_gen {
                    if let Some(lim) = app.context_limit {
                        let cap = session.lock().await.compact_cap_tokens();
                        let trigger = forge_core::auto_compact_trigger_tokens(
                            lim as u64,
                            cap,
                            AUTO_COMPACT_THRESHOLD,
                        );
                        if app.context_tokens > trigger {
                            let fill = app.context_tokens as f64 / lim as f64;
                            app.note(&format!(
                                "⚒ context {:.0}% full — auto-compacting",
                                fill * 100.0
                            ));
                            turn_gen += 1;
                            last_auto_compact_gen = turn_gen;
                            turn_handle = Some(spawn_compact(
                                &session,
                                &done_tx,
                                turn_gen,
                                &mut app,
                                &mut busy,
                                &mut busy_since,
                            ));
                        }
                    }
                }
            }
        }
        if busy {
            let t = (busy_since.elapsed().as_millis() / 60) as usize;
            if t != app.tick {
                app.tick = t;
                dirty = true;
            }
        }
        // Refresh shell-backed custom statusline widgets on their configured interval. Spawned
        // detached (not awaited, doesn't touch the session lock), so a slow or hanging user
        // command can never stall the render loop — a stuck one just keeps showing stale output.
        {
            let now = Instant::now();
            let due: Vec<String> = app
                .statusline_config
                .left
                .iter()
                .chain(app.statusline_config.center.iter())
                .chain(app.statusline_config.right.iter())
                .chain(app.statusline_config.extra_rows.iter().flatten())
                .filter_map(|widget| match widget {
                    forge_config::StatuslineWidget::Custom {
                        shell: Some(cmd),
                        refresh_secs,
                        ..
                    } if custom_widget_last_run
                        .get(cmd)
                        .map(|last| now.duration_since(*last) >= Duration::from_secs(*refresh_secs))
                        .unwrap_or(true) =>
                    {
                        Some(cmd.clone())
                    }
                    _ => None,
                })
                .collect();
            for cmd in due {
                custom_widget_last_run.insert(cmd.clone(), now);
                let tx = tx_custom.clone();
                tokio::spawn(async move {
                    const TIMEOUT: Duration = Duration::from_secs(5);
                    let (sh, sh_flag) = shell_widget_shell();
                    let Ok(child) = tokio::process::Command::new(sh)
                        .arg(sh_flag)
                        .arg(&cmd)
                        .stdin(std::process::Stdio::null())
                        .stdout(std::process::Stdio::piped())
                        .stderr(std::process::Stdio::null())
                        .kill_on_drop(true) // a timeout drops the future → child killed, not orphaned
                        .spawn()
                    else {
                        return;
                    };
                    if let Ok(Ok(out)) =
                        tokio::time::timeout(TIMEOUT, child.wait_with_output()).await
                    {
                        if out.status.success() {
                            let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
                            let _ = tx.send(UiMsg::Event(
                                forge_tui::PresenterEvent::CustomWidgetOutput { id: cmd, text },
                            ));
                        }
                    }
                });
            }
        }
        // Animate the effort slider's rainbow/pulse at XHigh even while idle.
        if app.effort_slider {
            let t = (anim_epoch.elapsed().as_millis() / 80) as usize;
            if t != app.tick {
                app.tick = t;
                dirty = true;
            }
        }
        // Blink the input cursor only when focused AND idle: solid for the first ~600ms after the
        // last keystroke, then a calm ~600ms square wave. Typing resets `last_input_at`, so the
        // block never flickers while you write. Unfocused → static hollow, so leave it alone.
        if !app.unfocused {
            let idle = last_input_at.elapsed().as_millis();
            let phase_off = idle >= 600 && ((idle - 600) / 600) % 2 == 1;
            if phase_off != app.cursor_hidden {
                app.cursor_hidden = phase_off;
                dirty = true;
            }
        }
        // Animate the command palette's / picker's / at-path picker's ease-in reveal while open.
        if app.palette.open && app.palette.anim < 1.0 {
            app.palette.tick_anim();
            dirty = true;
        }
        if app.at_picker.open && app.at_picker.anim < 1.0 {
            app.at_picker.tick_anim();
            dirty = true;
        }
        if app.picker.open && app.picker.anim < 1.0 {
            app.picker.tick_anim();
            dirty = true;
        }
        if app.mesh_overlay.open && app.mesh_overlay.anim_tick < app.mesh_overlay.settle_tick() {
            // Animate only until the reveal settles, then stop redrawing (no infinite spinner).
            app.mesh_overlay.anim_tick += 1;
            dirty = true;
        }
        if app.workflow.open && app.workflow.anim_tick < app.workflow.settle_tick() {
            // Same settle-then-stop reveal as the mesh inspector; while the run is busy the
            // normal spinner tick keeps the row spinners animating past this point.
            app.workflow.anim_tick += 1;
            dirty = true;
        }
        if app.usage_overlay.open {
            app.usage_overlay.anim_tick = app.usage_overlay.anim_tick.wrapping_add(1);
            dirty = true;
            // Auto-refresh data every ~3 s (180 ticks × 16 ms). `try_lock`, not a blocking
            // `.await`: this is the main render loop, and a busy turn parked in a permission/
            // question prompt holds this lock for the whole prompt — blocking here would wedge
            // the entire loop past the point where it could ever read the keystroke that answers
            // that prompt. On contention this tick's refresh is simply skipped and retried in ~3s.
            if app.usage_overlay.anim_tick % 180 == 1 {
                if let Ok(s) = session.try_lock() {
                    let (
                        (
                            month_usd,
                            by_model_5h,
                            by_model,
                            by_model_week,
                            (daily_cap, monthly_cap, weekly_cap),
                        ),
                        (session_in, session_out, session_usd),
                    ) = (
                        (
                            s.spend_this_month_usd(),
                            s.spend_by_model_5h(),
                            s.spend_by_model_today(),
                            s.spend_by_model_week(),
                            s.budget_caps(),
                        ),
                        s.session_usage_db(),
                    );
                    drop(s);
                    app.usage_overlay.month_usd = month_usd;
                    app.usage_overlay.session_usd = session_usd;
                    app.usage_overlay.session_in = session_in;
                    app.usage_overlay.session_out = session_out;
                    app.usage_overlay.by_model_5h = by_model_5h;
                    app.usage_overlay.by_model = by_model;
                    app.usage_overlay.by_model_week = by_model_week;
                    app.usage_overlay.daily_cap = daily_cap;
                    app.usage_overlay.weekly_cap = weekly_cap;
                    app.usage_overlay.monthly_cap = monthly_cap;
                    // bridge_stats scan can take seconds on large histories — fire it in the
                    // background and let the existing usage_load_rx receiver fill in the
                    // claude quota fields without stalling the event loop.
                    if usage_load_rx.is_none() {
                        let (tx, rx) = tokio::sync::oneshot::channel();
                        tokio::task::spawn_blocking(move || {
                            let _ = tx.send(bridge_stats::fetch());
                        });
                        usage_load_rx = Some(rx);
                    }
                }
            }
        }

        // Poll mesh background load (opened with loading=true; result populates when ready).
        if let Some(rx) = &mut mesh_load_rx {
            match rx.try_recv() {
                Ok(Some(overlay)) => {
                    let tick = app.mesh_overlay.anim_tick;
                    app.mesh_overlay = overlay;
                    app.mesh_overlay.anim_tick = tick;
                    mesh_load_rx = None;
                    dirty = true;
                }
                Ok(None) => {
                    app.mesh_overlay.open = false;
                    mesh_load_rx = None;
                    emit_text(
                        Some(&mut tui),
                        &mut app,
                        "mesh: auto-discovery routing is off (no model catalog) — nothing to inspect",
                    );
                    dirty = true;
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    app.mesh_overlay.open = false;
                    mesh_load_rx = None;
                    dirty = true;
                }
            }
        }
        // Poll usage background load (bridge stats; session data was already populated on open).
        if let Some(rx) = &mut usage_load_rx {
            match rx.try_recv() {
                Ok(bstats) => {
                    // `try_lock`: same reasoning as the remote-snapshot publish below — this is
                    // the main render loop, and a busy turn parked in a permission/question prompt
                    // holds this lock for the whole prompt. Worst case on contention is one overlay
                    // refresh using stale (empty) fractions, not a permanently wedged loop.
                    let fracs = session
                        .try_lock()
                        .map(|s| s.bridge_fractions())
                        .unwrap_or_default();
                    app.usage_overlay.claude_5h_in = bstats.claude_5h_in;
                    app.usage_overlay.claude_5h_out = bstats.claude_5h_out;
                    app.usage_overlay.claude_weekly_in = bstats.claude_weekly_in;
                    app.usage_overlay.claude_weekly_out = bstats.claude_weekly_out;
                    fill_subscription_pcts(&mut app.usage_overlay, &fracs, &bstats);
                    app.usage_overlay.loading = false;
                    usage_load_rx = None;
                    dirty = true;
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    app.usage_overlay.loading = false;
                    usage_load_rx = None;
                    dirty = true;
                }
            }
        }

        // `/voice`: animate the REC blink / spinner, tick the elapsed-time counter at 1Hz (mirrors
        // `turn_elapsed_secs` above), and sample the live RMS level into the waveform ring buffer.
        // Unconditional `dirty = true` while open, same convention as the mesh/usage overlays
        // above — the animation itself is the reason to redraw, not a side effect of other state.
        if let Some(v) = app.voice.as_mut() {
            v.anim_tick = v.anim_tick.wrapping_add(1);
            dirty = true;
            if matches!(v.phase, forge_tui::VoicePhase::Recording { .. }) {
                if let Some(started) = voice_started_at {
                    v.elapsed_secs = started.elapsed().as_secs();
                }
                if let Some(handle) = &voice_handle {
                    v.push_level(*handle.levels.borrow());
                }
            }
        }
        // Auto-close the error card after its timeout (also dismissible by any keypress above).
        if let Some(until) = voice_error_until {
            if Instant::now() >= until {
                app.voice = None;
                voice_error_until = None;
                dirty = true;
            }
        }
        // Poll the whisper-model download's progress (a `watch` channel — always holds the latest
        // value, so `has_changed`/`borrow_and_update` rather than `try_recv`).
        if let Some(rx) = &mut voice_download_progress_rx {
            if rx.has_changed().unwrap_or(false) {
                let (done, total) = *rx.borrow_and_update();
                if let Some(v) = app.voice.as_mut() {
                    if let forge_tui::VoicePhase::Downloading {
                        done_mb, total_mb, ..
                    } = &mut v.phase
                    {
                        *done_mb = done as f64 / 1_048_576.0;
                        *total_mb = total.map(|t| t as f64 / 1_048_576.0);
                    }
                }
                dirty = true;
            }
        }
        // Poll the download's completion: model landed + recording started (Ok), or a download/mic
        // failure (Err) — either way the overlay moves out of the `Downloading` phase.
        if let Some(rx) = &mut voice_download_done_rx {
            match rx.try_recv() {
                Ok(Ok(handle)) => {
                    voice_handle = Some(handle);
                    voice_started_at = Some(Instant::now());
                    voice_ptt_active = tui.push_voice_ptt();
                    app.voice = Some(forge_tui::VoiceOverlay::recording(voice_ptt_active));
                    voice_download_progress_rx = None;
                    voice_download_done_rx = None;
                    dirty = true;
                }
                Ok(Err(e)) => {
                    app.voice = Some(forge_tui::VoiceOverlay::error(format!("voice: {e}")));
                    voice_error_until = Some(Instant::now() + Duration::from_secs(2));
                    voice_model_path = None;
                    voice_download_progress_rx = None;
                    voice_download_done_rx = None;
                    dirty = true;
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    app.voice = None;
                    voice_download_progress_rx = None;
                    voice_download_done_rx = None;
                    dirty = true;
                }
            }
        }
        // Poll the transcription result (Enter or a held-then-released push-to-talk chord):
        // insert into `app.input` at the cursor and close the overlay, or show the failure.
        if let Some(rx) = &mut voice_transcript_rx {
            match rx.try_recv() {
                Ok(Ok(text)) => {
                    voice_transcript_rx = None;
                    app.voice = None;
                    let n = forge_tui::insert_voice_transcript(
                        &mut app.input,
                        &mut app.input_cursor,
                        &text,
                    );
                    if n == 0 {
                        app.note("voice: heard nothing");
                    } else {
                        app.note(&format!("voice: inserted {n} chars"));
                    }
                    dirty = true;
                }
                Ok(Err(e)) => {
                    voice_transcript_rx = None;
                    app.voice = Some(forge_tui::VoiceOverlay::error(format!("voice: {e}")));
                    voice_error_until = Some(Instant::now() + Duration::from_secs(2));
                    dirty = true;
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    voice_transcript_rx = None;
                    app.voice = None;
                    dirty = true;
                }
            }
        }

        // Push any finalized lines into native scrollback (above the pinned live region). While
        // remote control is on, also fold them into the transcript ring buffer so the phone's
        // snapshot mirrors the conversation tail, then broadcast the snapshot.
        if remote.is_some() {
            let flushed = app.drain_flush_remote();
            if !flushed.is_empty() {
                tui.insert_lines(flushed);
                dirty = true;
            }
            if dirty || busy {
                let exposure = remote
                    .as_ref()
                    .map(|rc| {
                        remote::exposure_label(
                            rc.tunnel,
                            rc.url.tls_fingerprint.is_some(),
                            rc.tls_failed(),
                        )
                    })
                    .unwrap_or_default();
                // `try_lock`, NEVER a blocking `.await` here: a busy turn parked in a permission/
                // question prompt holds this exact lock for the whole prompt, and this is the main
                // render loop itself — blocking here wedges the ENTIRE loop mid-iteration, past the
                // point where it would ever poll for a new keystroke again, so even Esc/Ctrl-C (or
                // remote's own Interrupt) can never be read to unblock the turn. The id is stable
                // for a session's lifetime in practice, so a stale reuse while contended costs
                // nothing.
                if let Ok(s) = session.try_lock() {
                    cached_session_id = s.session_id().to_string();
                }
                let mut snap = build_snapshot_frame(
                    &app,
                    SnapshotIdentity {
                        session_id: &cached_session_id,
                        title: "",
                        cwd: &remote_cwd,
                        worktree: None,
                        exposure,
                    },
                    remote_copy_text.clone(),
                    prompt_seq,
                    remote_notes.clone(),
                    // Candidate carries the LAST revision so the equality compare below sees
                    // only real state changes; bumped just before an actual send.
                    remote_revision,
                );
                // Change-only broadcast: while busy this branch runs every 16ms, and
                // `watch::send` notifies every subscriber unconditionally — without this
                // compare each client got ~60 identical JSON frames/s for a whole turn.
                if last_remote_snap.as_ref() != Some(&snap) {
                    remote_revision += 1;
                    snap.revision = remote_revision;
                    last_remote_snap = Some(snap.clone());
                    if let Some(rc) = remote.as_ref() {
                        // `broadcast` (not a bare `send`): the frame must also land in the
                        // replay log so a reconnecting page gets exactly what it missed.
                        rc.broadcast(snap);
                    }
                }
            }
        } else {
            let flushed = app.drain_flush();
            if !flushed.is_empty() {
                tui.insert_lines(flushed);
                dirty = true;
            }
        }
        // Adaptive frame pacing. When the user is actively interacting (a key/paste was handled
        // this iteration) and no turn is streaming, loop back quickly so typing/selection in the
        // palette, picker, and approve prompts feels immediate instead of capped at ~60fps. Idle or
        // mid-stream → a full ~16ms frame keeps CPU low and the spinner smooth.
        let snappy = dirty && !busy;
        tokio::time::sleep(Duration::from_millis(if snappy { 3 } else { 16 })).await;
    }
    {
        let (hooks, sid) = {
            let s = session.lock().await;
            // Save the final view on clean exit so resuming this session restores the screen.
            if let Some(json) = app.view_snapshot_json() {
                s.save_view_snapshot(&json);
            }
            (s.hooks().to_vec(), s.session_id().to_string())
        };
        forge_core::hooks::run_session_hooks(&hooks, forge_config::HookEvent::SessionEnd, &sid)
            .await;
    }
    Ok(())
}

/// Push-to-talk hold threshold: a chord release faster than this is a "tap" (leave the overlay in
/// ordinary toggle mode — Enter/Esc/r); at or past it is a "hold" (auto-stop + transcribe on
/// release). Pure so the decision is unit-testable in isolation from any terminal/async plumbing.
pub(crate) const VOICE_PTT_HOLD_MS: u128 = 400;

/// See [`VOICE_PTT_HOLD_MS`].
pub(crate) fn voice_is_hold(held_ms: u128) -> bool {
    held_ms >= VOICE_PTT_HOLD_MS
}

/// Wire a freshly-dispatched `/voice` into the loop-local recorder/download state. `App::voice`
/// (the rendering-facing state) is already set by `dispatch_command` — this only handles the real
/// system resources, which must live loop-local so `App` stays `Clone + Default`. Shared by every
/// place `/voice` can be triggered from: the palette, a typed `/voice` line, remote input, and the
/// Ctrl+V shortcut. Resets every voice-related loop-local first, so no state from a previous voice
/// session can bleed through.
#[allow(clippy::too_many_arguments)]
fn apply_voice_start(
    start: VoiceStart,
    tui: &mut forge_tui::Tui,
    app: &mut forge_tui::App,
    voice_handle: &mut Option<forge_voice::RecordingHandle>,
    voice_model_path: &mut Option<std::path::PathBuf>,
    voice_download_progress_rx: &mut Option<tokio::sync::watch::Receiver<(u64, Option<u64>)>>,
    voice_download_done_rx: &mut Option<
        tokio::sync::oneshot::Receiver<std::result::Result<forge_voice::RecordingHandle, String>>,
    >,
    voice_started_at: &mut Option<std::time::Instant>,
    voice_error_until: &mut Option<std::time::Instant>,
    voice_ptt_active: &mut bool,
) {
    *voice_handle = None;
    *voice_model_path = None;
    *voice_download_progress_rx = None;
    *voice_download_done_rx = None;
    *voice_started_at = None;
    *voice_error_until = None;
    if *voice_ptt_active {
        tui.pop_voice_ptt();
        *voice_ptt_active = false;
    }
    match start {
        VoiceStart::Recording { handle, model_path } => {
            *voice_ptt_active = tui.push_voice_ptt();
            if let Some(v) = app.voice.as_mut() {
                v.phase = forge_tui::VoicePhase::Recording {
                    ptt_active: *voice_ptt_active,
                };
            }
            *voice_handle = Some(handle);
            *voice_model_path = Some(model_path);
            *voice_started_at = Some(std::time::Instant::now());
        }
        VoiceStart::Downloading {
            model_path,
            progress_rx,
            done_rx,
        } => {
            *voice_model_path = Some(model_path);
            *voice_download_progress_rx = Some(progress_rx);
            *voice_download_done_rx = Some(done_rx);
        }
        VoiceStart::Error => {
            // `app.voice` is already the error card (set by `dispatch_command`).
            *voice_error_until =
                Some(std::time::Instant::now() + std::time::Duration::from_secs(2));
        }
    }
}

/// Stop the recording and kick off transcription in the background — shared by the Enter key
/// (toggle mode) and a held-then-released push-to-talk chord. Moves the overlay into
/// `VoicePhase::Transcribing`; the tick loop polls the returned receiver for the result.
fn start_voice_transcribe(
    app: &mut forge_tui::App,
    handle: forge_voice::RecordingHandle,
    model_path: std::path::PathBuf,
) -> tokio::sync::oneshot::Receiver<forge_voice::Result<String>> {
    if let Some(v) = app.voice.as_mut() {
        v.phase = forge_tui::VoicePhase::Transcribing;
    }
    let config = forge_config::load().unwrap_or_default();
    let language = (config.voice.language != "auto").then_some(config.voice.language);
    let (tx, rx) = tokio::sync::oneshot::channel();
    tokio::task::spawn_blocking(move || {
        let result = (|| -> forge_voice::Result<String> {
            let samples = handle.stop()?;
            let transcriber = forge_voice::Transcriber::load(&model_path)?;
            transcriber.transcribe(&samples, language.as_deref())
        })();
        let _ = tx.send(result);
    });
    rx
}

/// `/loop` runtime state: the generation of the in-flight loop turn and how many iterations have
/// run, so completion can be detected and capped.
pub(crate) struct LoopState {
    gen: u64,
    iter: usize,
}

/// Iteration cap so a loop that never signals completion can't run forever.
pub(crate) const LOOP_MAX_ITERS: usize = 25;

/// Context-fill fraction above which a turn-end auto-compact fires (context-compaction.md).
pub(crate) const AUTO_COMPACT_THRESHOLD: f64 = 0.80;

/// The token the model is told to emit when the looped task is fully complete.
pub(crate) const LOOP_DONE_SENTINEL: &str = "LOOP_COMPLETE";

/// Guidance injected on every loop turn: make progress, and signal completion explicitly.
pub(crate) const LOOP_GUIDANCE: &str = "You are running in an autonomous loop. Make concrete progress on the \
task each turn. When — and ONLY when — the task is fully complete, end your final message with \
the token LOOP_COMPLETE on its own line. While work remains, keep going and do NOT emit that token.";

/// Decide whether a loop should stop after a turn. Returns `Some(reason)` to stop (shown to the
/// user), or `None` to run another iteration. Pure so it's unit-testable.
pub(crate) fn loop_stop_reason(last_assistant: Option<&str>, iter: usize) -> Option<&'static str> {
    if last_assistant.is_some_and(|t| t.contains(LOOP_DONE_SENTINEL)) {
        Some("◆ loop complete")
    } else if iter >= LOOP_MAX_ITERS {
        Some("◆ loop stopped — hit the iteration cap")
    } else {
        None
    }
}

/// `/goal` runtime state: the generation of the in-flight goal turn, how many iterations have
/// run, and how many tasks were done as of the last turn — so stalls (no task progress) can be
/// detected alongside completion and the iteration cap.
pub(crate) struct GoalState {
    gen: u64,
    iter: usize,
    prev_done: usize,
    no_progress: usize,
    goal: String,
}

/// Absolute iteration ceiling so a goal that never signals completion can't run forever.
pub(crate) const GOAL_MAX_ITERS: usize = 200;

/// Consecutive turns with no task-list progress before a goal is declared wedged and stopped.
pub(crate) const GOAL_NO_PROGRESS_MAX: usize = 6;

/// Guidance injected on every goal turn: work the tracked plan, never stop for approval, and
/// signal completion explicitly.
pub(crate) const GOAL_GUIDANCE: &str = "You are in autonomous goal mode. Keep working through the \
tracked task plan (update_tasks) one item at a time until the entire goal is met. Never stop to \
ask for approval — you have standing authorization. When (and only when) every task is Done and \
the goal is fully satisfied, reply with one concise final response that states the goal is complete \
and briefly summarizes what was done. Do not emit control sentinels or repeated completion messages.";

/// The prompt each re-drive turn is given once the goal is running autonomously.
pub(crate) const GOAL_CONTINUE_PROMPT: &str = "Continue the goal. Commit/push/PR any finished \
work, then take the single highest-value not-done task and complete it end to end. Keep the \
update_tasks plan current.";

/// Legacy completion marker accepted only as an exact standalone reply. New goal guidance asks
/// for a normal final response and completion is otherwise inferred from the tracked task plan.
pub(crate) fn is_goal_complete_marker(text: Option<&str>) -> bool {
    text.is_some_and(|text| text.trim() == "GOAL COMPLETE")
}

pub(crate) const GOAL_COMPLETE_REASON: &str = "🎯 goal complete";

pub(crate) fn is_goal_complete_reason(reason: &str) -> bool {
    reason == GOAL_COMPLETE_REASON || reason == "🎯 goal complete — all tasks done"
}

/// Decide whether a goal should stop after a turn. Returns `Some(reason)` to stop (shown to the
/// user), or `None` to run another iteration. Pure so it's unit-testable.
pub(crate) fn goal_stop_reason(
    said_complete: bool,
    done: usize,
    total: usize,
    iter: usize,
    no_progress: usize,
) -> Option<&'static str> {
    if said_complete {
        Some(GOAL_COMPLETE_REASON)
    } else if total > 0 && done == total {
        Some("🎯 goal complete — all tasks done")
    } else if iter >= GOAL_MAX_ITERS {
        Some("🎯 goal stopped — iteration ceiling")
    } else if no_progress >= GOAL_NO_PROGRESS_MAX {
        Some("🎯 goal stalled — no task progress, stopping")
    } else {
        None
    }
}

/// Echo a prompt + spawn the turn task (shared by normal submit and the `//` literal escape).
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_turn(
    prompt: &str,
    session: &Arc<tokio::sync::Mutex<Session>>,
    done_tx: &std::sync::mpsc::Sender<u64>,
    gen: u64,
    app: &mut forge_tui::App,
    busy: &mut bool,
    busy_since: &mut std::time::Instant,
) -> tokio::task::JoinHandle<()> {
    app.on_turn_start();
    app.submit_user(prompt);
    app.done = false;
    app.tick = 0;
    *busy = true;
    *busy_since = std::time::Instant::now();
    let s = session.clone();
    let dt = done_tx.clone();
    let prompt = prompt.to_string();
    tokio::spawn(async move {
        // DoneGuard fires on the way out — normal return, panic unwind, OR abort (interrupt) —
        // so the UI can never stay stuck "working". It carries this turn's generation.
        let _done = DoneGuard(dt, gen);
        let mut sess = s.lock().await;
        if let Err(e) = sess.run_turn(&prompt).await {
            sess.notify_error(&format!("turn failed: {e}"));
        }
    })
}

/// Like [`spawn_turn`] but runs an expanded command/skill: prepends `guidance` and biases routing
/// with the `tier` hint. The displayed user line is the original `/command` (echoed by the
/// dispatcher), so the model receives the expanded `prompt` while the transcript shows the turn.
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_turn_with(
    prompt: String,
    guidance: Vec<String>,
    tier: Option<forge_types::TaskTier>,
    session: &Arc<tokio::sync::Mutex<Session>>,
    done_tx: &std::sync::mpsc::Sender<u64>,
    gen: u64,
    app: &mut forge_tui::App,
    busy: &mut bool,
    busy_since: &mut std::time::Instant,
) -> tokio::task::JoinHandle<()> {
    app.on_turn_start();
    app.submit_user(&prompt);
    app.done = false;
    app.tick = 0;
    *busy = true;
    *busy_since = std::time::Instant::now();
    let s = session.clone();
    let dt = done_tx.clone();
    tokio::spawn(async move {
        let _done = DoneGuard(dt, gen);
        let mut sess = s.lock().await;
        if let Err(e) = sess.run_turn_with(&prompt, &guidance, tier).await {
            sess.notify_error(&format!("turn failed: {e}"));
        }
    })
}

/// Spawn `/compact` as a background task (it makes a cheap model call): the spinner ticks while the
/// older transcript is summarized, exactly like a turn.
pub(crate) fn spawn_compact(
    session: &Arc<tokio::sync::Mutex<Session>>,
    done_tx: &std::sync::mpsc::Sender<u64>,
    gen: u64,
    app: &mut forge_tui::App,
    busy: &mut bool,
    busy_since: &mut std::time::Instant,
) -> tokio::task::JoinHandle<()> {
    app.done = false;
    app.tick = 0;
    *busy = true;
    *busy_since = std::time::Instant::now();
    let s = session.clone();
    let dt = done_tx.clone();
    tokio::spawn(async move {
        let _done = DoneGuard(dt, gen);
        let mut sess = s.lock().await;
        if let Err(e) = sess.compact(false).await {
            sess.notify_error(&format!("compact failed: {e}"));
        }
    })
}

/// Keys inside the workflow view's Enter-zoom transcript: mirrors the activity viewer's scrolling
/// (↑↓/PgUp/PgDn/Home/End, j/k/g/G) plus ←→/Tab to switch agents. An upward scroll first snaps
/// the stored offset from the "tail" sentinel down to the real max (recorded by the render path),
/// so scrolling back out of follow mode moves on the first keypress instead of unwinding a
/// `usize::MAX / 2` sentinel one line at a time.
pub(crate) fn workflow_zoom_key(app: &mut forge_tui::App, key: forge_tui::KeyKind) {
    use forge_tui::KeyKind;
    let max = app
        .workflow
        .zoom_geom
        .get()
        .map(|(wrapped_len, body_h)| wrapped_len.saturating_sub(body_h as usize));
    let clamp = |scroll: usize| max.map_or(scroll, |m| scroll.min(m));
    match key {
        KeyKind::Esc | KeyKind::Char('q') => app.workflow.zoom = None,
        KeyKind::Up | KeyKind::Char('k') => {
            if let Some(z) = app.workflow.zoom.as_mut() {
                z.follow = false;
                z.scroll = clamp(z.scroll).saturating_sub(1);
            }
        }
        KeyKind::Down | KeyKind::Char('j') | KeyKind::Char(' ') => {
            if let Some(z) = app.workflow.zoom.as_mut() {
                z.scroll = clamp(z.scroll).saturating_add(1);
            }
            app.workflow.zoom_refollow_at_tail();
        }
        KeyKind::PageUp => {
            if let Some(z) = app.workflow.zoom.as_mut() {
                z.follow = false;
                z.scroll = clamp(z.scroll).saturating_sub(10);
            }
        }
        KeyKind::PageDown => {
            if let Some(z) = app.workflow.zoom.as_mut() {
                z.scroll = clamp(z.scroll).saturating_add(10);
            }
            app.workflow.zoom_refollow_at_tail();
        }
        KeyKind::Home | KeyKind::Char('g') => {
            if let Some(z) = app.workflow.zoom.as_mut() {
                z.follow = false;
                z.scroll = 0;
            }
        }
        KeyKind::End | KeyKind::Char('G') => {
            if let Some(z) = app.workflow.zoom.as_mut() {
                z.follow = true;
                z.scroll = usize::MAX / 2;
            }
        }
        KeyKind::Left => {
            app.workflow.move_selection(-1);
            app.workflow.zoom = Some(Default::default());
        }
        KeyKind::Right | KeyKind::Tab => {
            app.workflow.move_selection(1);
            app.workflow.zoom = Some(Default::default());
        }
        _ => {}
    }
}

/// Spawn `/workflow run <name>` as a background task (docs/rfcs/forge-workflow.md): runs a saved
/// script directly, no authoring turn, same busy/spinner/interrupt semantics as a normal turn.
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_saved_workflow(
    session: &Arc<tokio::sync::Mutex<Session>>,
    done_tx: &std::sync::mpsc::Sender<u64>,
    gen: u64,
    app: &mut forge_tui::App,
    busy: &mut bool,
    busy_since: &mut std::time::Instant,
    name: String,
    args: serde_json::Value,
) -> tokio::task::JoinHandle<()> {
    app.done = false;
    app.tick = 0;
    *busy = true;
    *busy_since = std::time::Instant::now();
    let s = session.clone();
    let dt = done_tx.clone();
    tokio::spawn(async move {
        let _done = DoneGuard(dt, gen);
        let mut sess = s.lock().await;
        if let Err(e) = sess.run_saved_workflow(&name, args).await {
            sess.notify_error(&format!("workflow '{name}' failed: {e}"));
        }
    })
}

/// Spawn `/duel <task>` as a background task (docs/features/duel.md): same busy/spinner/interrupt
/// semantics as a normal turn. Unlike `run_saved_workflow`, the result isn't just a presenter
/// event trail — the finished report + still-alive worktree guards must reach the render loop so
/// it can open a picker over the candidates, so they're written into `pending_duel` for the
/// done-signal drain to pick up.
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_duel(
    session: &Arc<tokio::sync::Mutex<Session>>,
    done_tx: &std::sync::mpsc::Sender<u64>,
    gen: u64,
    app: &mut forge_tui::App,
    busy: &mut bool,
    busy_since: &mut std::time::Instant,
    task: String,
    pending_duel: Arc<std::sync::Mutex<PendingDuel>>,
) -> tokio::task::JoinHandle<()> {
    app.done = false;
    app.tick = 0;
    *busy = true;
    *busy_since = std::time::Instant::now();
    let s = session.clone();
    let dt = done_tx.clone();
    tokio::spawn(async move {
        let _done = DoneGuard(dt, gen);
        let mut sess = s.lock().await;
        match sess.run_duel(&task).await {
            Ok(result) => *pending_duel.lock().unwrap() = Some(result),
            Err(e) => sess.notify_error(&format!("duel failed: {e}")),
        }
    })
}

/// Who/where a snapshot frame describes — the per-session identity fields of
/// [`remote::Snapshot`] that the driving loop (TUI render loop or a headless `forge serve`
/// session driver) knows and `App` doesn't.
pub(crate) struct SnapshotIdentity<'a> {
    pub session_id: &'a str,
    /// Session display title (v6). Empty when unnamed.
    pub title: &'a str,
    pub cwd: &'a str,
    /// The isolated worktree the session runs in (v6), if any.
    pub worktree: Option<&'a str>,
    /// "loopback" | "LAN" | "public (provider)" — see [`remote::exposure_label`].
    pub exposure: String,
}

/// Build one wire [`remote::Snapshot`] frame from the App's remote projection. The ONE snapshot
/// producer shared by `run_chat_tui`'s broadcast block and the headless `forge serve` session
/// driver, so both paths serialize the identical shape. `revision` should carry the LAST
/// broadcast revision — the caller compares for change and bumps it just before an actual send.
pub(crate) fn build_snapshot_frame(
    app: &forge_tui::App,
    ident: SnapshotIdentity<'_>,
    copy_text: Option<String>,
    prompt_seq: u64,
    notes: Vec<String>,
    revision: u64,
) -> remote::Snapshot {
    let view = app.remote_snapshot();
    remote::Snapshot {
        protocol: remote::PROTOCOL_VERSION,
        session_id: ident.session_id.to_string(),
        title: ident.title.to_string(),
        cwd: ident.cwd.to_string(),
        worktree: ident.worktree.map(str::to_string),
        exposure: ident.exposure,
        busy: view.busy,
        done: view.done,
        temper: view.temper,
        effort: view
            .effort
            .map(forge_types::EffortLevel::as_str)
            .unwrap_or("medium")
            .to_string(),
        tier: view.tier,
        model: view.model,
        cost_usd: view.cost_usd,
        context_tokens: view.context_tokens,
        context_limit: view.context_limit,
        streaming: view.streaming,
        transcript: view.transcript,
        tasks: view
            .tasks
            .iter()
            .map(|t| remote::SnapTask {
                title: t.title.clone(),
                status: match t.status {
                    forge_types::TodoStatus::Pending => "pending",
                    forge_types::TodoStatus::InProgress => "in_progress",
                    forge_types::TodoStatus::Done => "done",
                }
                .to_string(),
            })
            .collect(),
        subagents: view
            .subagents
            .iter()
            .map(|s| remote::SnapSubagent {
                agent: s.agent.clone(),
                task: s.task.clone(),
                model: s.model.clone(),
                last: s.last.clone(),
                done: s.done,
                cost: s.cost,
            })
            .collect(),
        queued: view.queued,
        permission_prompt: view.permission_prompt,
        question: view.question,
        question_options: view
            .question_options
            .iter()
            .map(|o| remote::SnapOption {
                label: o.label.clone(),
                description: o.description.clone(),
            })
            .collect(),
        question_allow_other: view.question_allow_other,
        // The generic overlay projection: whatever modal surface owns the keyboard
        // (palette / any picker / config / usage / mesh / workflow).
        overlay: app.remote_overlay().map(map_overlay_snapshot),
        diff: view.diff.map(map_diff_snapshot),
        plan: view.plan.map(|p| remote::SnapPlan {
            title: p.title,
            steps: p
                .steps
                .into_iter()
                .map(|s| remote::SnapPlanStep {
                    title: s.title,
                    detail: s.detail,
                })
                .collect(),
            notes: p.notes,
        }),
        suggested_prompt: view.suggested_prompt,
        copy_text,
        prompt_seq,
        notes,
        revision,
        resync: false,
        closed: false,
    }
}

/// Map the TUI-side diff projection into the remote wire type (same split as the overlay).
pub(crate) fn map_diff_snapshot(d: forge_tui::DiffSnapshot) -> remote::SnapDiff {
    remote::SnapDiff {
        pending: d.pending,
        files: d
            .files
            .into_iter()
            .map(|f| remote::SnapDiffFile {
                path: f.path,
                kind: f.kind,
                binary: f.binary,
                adds: f.adds,
                dels: f.dels,
                hunks: f
                    .hunks
                    .into_iter()
                    .map(|h| remote::SnapDiffHunk {
                        header: h.header,
                        lines: h.lines,
                    })
                    .collect(),
                skipped_lines: f.skipped_lines,
            })
            .collect(),
        skipped_files: d.skipped_files,
    }
}

/// Map the TUI-side overlay projection into the remote wire type (kept apart so `forge-tui`
/// never depends on the server module — same split as `RemoteSnapshot` → `Snapshot`).
pub(crate) fn map_overlay_snapshot(o: forge_tui::OverlaySnapshot) -> remote::SnapOverlay {
    remote::SnapOverlay {
        kind: o.kind,
        title: o.title,
        rows: o
            .rows
            .into_iter()
            .map(|r| remote::SnapRow {
                id: r.id,
                label: r.label,
                detail: r.detail,
                selected: r.selected,
                group: r.group,
            })
            .collect(),
        selected: o.selected,
        filter: o.filter,
        free_text: o.free_text,
        body: o.body,
    }
}

/// The next input event for the key loop: remote-injected keys first (so a remote overlay
/// commit — cursor move + synthesized Enter — is never interleaved by local typing), then the
/// terminal's own events. Remote keys become plain [`forge_tui::InputEvent::Key`]s here, so from
/// this point on they are indistinguishable from local keystrokes — the ONE code path both take.
fn next_input_event(
    remote_keys: &mut std::collections::VecDeque<forge_tui::KeyKind>,
    tui: &mut forge_tui::Tui,
) -> Result<Option<forge_tui::InputEvent>> {
    if let Some(k) = remote_keys.pop_front() {
        return Ok(Some(forge_tui::InputEvent::Key(k)));
    }
    tui.poll_event().context("reading input")
}

/// True when any modal surface owns the keyboard — the same set `remote_overlay()` projects.
pub(crate) fn any_remote_modal_open(app: &forge_tui::App) -> bool {
    app.workflow.open
        || app.config_editor.open
        || app.palette.open
        || app.usage_overlay.open
        || app.mesh_overlay.open
        || app.at_picker.open
        || app.picker.open
}

/// A remote overlay verb, decoded from [`remote::RemoteInput`] by the drain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RemoteOverlayOp {
    /// Move the cursor onto the row with this id, then commit it (synthesized Enter).
    Select(String),
    /// Move the cursor by this many rows (negative = up), as repeated ↑/↓ keys.
    Nav(i32),
    /// Replace the overlay's filter/query text (or the value being edited, for free-text).
    Filter(String),
    /// Close the overlay (Esc) — a no-op when nothing modal is open.
    Cancel,
}

/// Apply a remote overlay verb to the TOP-MOST open overlay (same precedence as
/// `App::remote_overlay`) and return the keystrokes to inject through the normal key path.
/// Select = set the cursor to the row with that id, then Enter — so a remotely committed picker
/// produces the identical `DispatchOutcome` handling a local Enter does. All mutations here are
/// cursor/filter state only; every side effect still happens in the shared key path.
pub(crate) fn apply_overlay_input(
    app: &mut forge_tui::App,
    op: RemoteOverlayOp,
) -> Vec<forge_tui::KeyKind> {
    use forge_tui::KeyKind as K;
    match op {
        RemoteOverlayOp::Cancel => {
            if any_remote_modal_open(app) {
                vec![K::Esc]
            } else {
                Vec::new()
            }
        }
        RemoteOverlayOp::Nav(delta) => {
            if !any_remote_modal_open(app) || delta == 0 {
                return Vec::new();
            }
            let key = if delta < 0 { K::Up } else { K::Down };
            // Bounded: a hostile frame can't queue an unbounded key storm.
            vec![key; delta.unsigned_abs().min(100) as usize]
        }
        RemoteOverlayOp::Filter(text) => {
            if app.workflow.open || app.usage_overlay.open || app.mesh_overlay.open {
                // Informational overlays have no filter.
            } else if app.config_editor.open {
                if app.config_editor.editing.is_some() {
                    app.config_editor.editing = Some(text);
                } else {
                    app.config_editor.filter = text;
                    app.config_editor.selected = 0;
                }
            } else if app.palette.open {
                // Mirror local typing: the palette query IS the input line's slash token.
                app.input = format!("/{text}");
                app.input_cursor = app.input.len();
                app.palette.query = text;
                app.palette.selected = 0;
                app.palette.clamp();
            } else if app.at_picker.open {
                app.at_picker.query = text;
                app.at_picker.selected = 0;
            } else if app.picker.open {
                app.picker.query = text;
                app.picker.selected = 0;
                app.picker.clamp();
            }
            Vec::new()
        }
        RemoteOverlayOp::Select(id) => {
            if app.workflow.open {
                if let Some(idx) = app.workflow.rows.iter().position(|r| r.id == id) {
                    app.workflow.selected = idx;
                }
                Vec::new() // Enter would zoom a transcript only the host can see
            } else if app.config_editor.open {
                if app.config_editor.editing.is_some() {
                    return Vec::new(); // committing the edit is the free-text box's job
                }
                let matches = app.config_editor.matches();
                if let Some(pos) = matches
                    .iter()
                    .position(|&i| app.config_editor.rows[i].path == id)
                {
                    app.config_editor.selected = pos;
                    return vec![K::Enter];
                }
                Vec::new()
            } else if app.palette.open {
                let names: Vec<String> =
                    app.palette.matches().into_iter().map(|e| e.name).collect();
                if let Some(idx) = names.iter().position(|n| *n == id) {
                    app.palette.selected = idx;
                    // A leading `/command` input line is what makes the palette's Enter
                    // dispatch (vs. accept-in-place) — materialize the pick exactly as typed.
                    app.input = format!("/{id}");
                    app.input_cursor = app.input.len();
                    return vec![K::Enter];
                }
                Vec::new()
            } else if app.usage_overlay.open {
                Vec::new() // informational — rows aren't selectable
            } else if app.mesh_overlay.open {
                if let Some(idx) = app
                    .mesh_overlay
                    .candidates
                    .iter()
                    .position(|c| c.model == id)
                {
                    app.mesh_overlay.cursor = idx;
                }
                Vec::new() // browsing highlight only, same as local ↑/↓
            } else if app.at_picker.open {
                if let Some(idx) = app.at_picker.matches().iter().position(|p| **p == id) {
                    app.at_picker.selected = idx;
                    return vec![K::Enter];
                }
                Vec::new()
            } else if app.picker.open {
                if let Some(idx) = app.picker.matches().iter().position(|r| r.id == id) {
                    app.picker.selected = idx;
                    return vec![K::Enter];
                }
                Vec::new()
            } else {
                Vec::new()
            }
        }
    }
}

/// Append a remote-facing notice (`Snapshot::notes`), keeping the ring bounded. These are state,
/// not events — `watch` coalescing can drop intermediate snapshots, so a note must survive until
/// the page has had a chance to render it.
pub(crate) fn push_remote_note(notes: &mut Vec<String>, msg: &str) {
    const MAX_REMOTE_NOTES: usize = 8;
    notes.push(msg.to_string());
    while notes.len() > MAX_REMOTE_NOTES {
        notes.remove(0);
    }
}

/// Prefix a remote prompt with the pending uploaded-text-file mentions (drained), so
/// `expand_at_files` inlines their contents exactly like a locally typed `@path`.
pub(crate) fn prepend_attach_mentions(mentions: &mut Vec<String>, text: String) -> String {
    if mentions.is_empty() {
        return text;
    }
    let m = mentions
        .drain(..)
        .map(|p| format!("@{p}"))
        .collect::<Vec<_>>()
        .join(" ");
    format!("{m}\n{text}")
}

/// Handle a [`remote::RemoteInput::Attach`] (the delivery leg of `POST /api/upload`): an image
/// becomes vision input on the session's next turn; a text file a pending `@path` mention.
///
/// The path is confined to the session's `.forge/uploads/` scratch area (canonicalized — no
/// symlink or `..` escape): `Attach` exists only to deliver uploads, so a WS client injecting
/// an arbitrary host path (`~/.ssh/id_rsa`) is refused with a note instead of read.
/// Confine an upload path to `<cwd>/.forge/uploads/` (canonicalized — no symlink/`..` escape).
/// Shared by [`handle_remote_attach`] (the ambient `Attach` input) and
/// [`resolve_prompt_attachments`] (the explicit, message-correlated attachment list on a
/// `Prompt`) — both exist only to deliver `POST /api/upload` results, so an arbitrary host path
/// (e.g. a WS client probing for secret files) must be refused either way.
fn remote_attach_confined(path: &str, cwd: &str) -> bool {
    let root = std::path::Path::new(cwd).join(".forge").join("uploads");
    std::fs::canonicalize(path)
        .ok()
        .zip(std::fs::canonicalize(&root).ok())
        .map(|(p, r)| p.starts_with(&r))
        .unwrap_or(false)
}

pub(crate) async fn handle_remote_attach(
    session: &Arc<tokio::sync::Mutex<Session>>,
    app: &mut forge_tui::App,
    mentions: &mut Vec<String>,
    cwd: &str,
    path: String,
    image: bool,
) {
    if !remote_attach_confined(&path, cwd) {
        app.note("⚠ attach ignored — not a file from this session's upload area");
        return;
    }
    if image {
        match crate::image_input::load_image_file(&path) {
            Ok((att, label)) => {
                session.lock().await.attach_images(vec![att]);
                // Also record a `@path` mention, exactly like the non-image branch below: the
                // vision attachment only rides THIS turn's provider call (`attach_images` is
                // transient), so without a durable mention the image reference never reaches
                // persisted history — it renders fine live, then silently vanishes after any
                // history reload (new device, app restart). The mention gives the mobile client
                // something resolvable to detect and re-render on reload via `GET /api/upload`.
                mentions.push(path);
                app.note(&format!(
                    "🖼 image attached ({label}) — rides the next prompt"
                ));
            }
            Err(e) => app.note(&format!("⚠ image attach failed: {e}")),
        }
    } else {
        let name = std::path::Path::new(&path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.clone());
        mentions.push(path);
        app.note(&format!(
            "📎 attached {name} — included with the next prompt"
        ));
    }
}

/// Resolve a [`remote::RemoteInput::Prompt`]'s explicit, message-correlated `attachments` list
/// (mobile-upload-race fix): when non-empty it is AUTHORITATIVE for this turn, so any stale
/// `pending_images` left over from an unrelated `Attach` (e.g. an image already uploading for a
/// different, adjacent message) is discarded first — it must never leak into this turn — then
/// each listed attachment is resolved fresh with the same confinement check
/// [`handle_remote_attach`] uses. Images ride straight onto the session; non-image files come
/// back as plain paths (the caller prepends them onto the prompt text as `@path` mentions via
/// [`prepend_attach_mentions`] itself, at the point where the old ambient mentions were applied —
/// this function never touches `text`, so a `//`-escape or `/command` dispatched off the SAME
/// prompt still parses cleanly).
///
/// An empty list (older client, or a plain message with genuinely no attachments) is a no-op that
/// returns an empty `Vec` without touching any session state — callers fall back to exactly the
/// pre-existing ambient `Attach`-then-`Prompt` behavior.
pub(crate) async fn resolve_prompt_attachments(
    session: &Arc<tokio::sync::Mutex<Session>>,
    app: &mut forge_tui::App,
    remote_notes: &mut Vec<String>,
    cwd: &str,
    attachments: Vec<remote::PromptAttachment>,
) -> Vec<String> {
    if attachments.is_empty() {
        return Vec::new();
    }
    // Drop, don't use: whatever's ambiently pending belongs to no turn now that an explicit,
    // authoritative list has arrived for THIS one.
    let _ = session.lock().await.take_pending_images();

    let mut mentions = Vec::new();
    for att in attachments {
        if !remote_attach_confined(&att.path, cwd) {
            tracing::warn!(
                path = %att.path,
                cwd = %cwd,
                "prompt attachment rejected: outside session's upload area"
            );
            push_remote_note(
                remote_notes,
                "⚠ attach ignored — not a file from this session's upload area",
            );
            continue;
        }
        if att.image {
            match crate::image_input::load_image_file(&att.path) {
                Ok((img, label)) => {
                    tracing::info!(path = %att.path, %label, "prompt image attachment resolved");
                    session.lock().await.attach_images(vec![img]);
                    app.note(&format!("🖼 image attached ({label}) — rides this prompt"));
                }
                Err(e) => {
                    tracing::warn!(path = %att.path, error = %e, "prompt image attachment failed to load");
                    app.note(&format!("⚠ image attach failed: {e}"));
                }
            }
        } else {
            mentions.push(att.path);
        }
    }
    mentions
}

/// Start or stop remote control in response to `/remote`. On: bind the server (LAN-reachable by
/// default, loopback with `--local`, or piped through a public tunnel with `--anywhere`), print
/// the connect URL + a scan-to-connect QR code into scrollback, and light the statusline
/// indicator. Off: drop the handle (stops the server + tunnel, frees the port) and clear the
/// indicator. Idempotent: `/remote` toggles, so running it again turns it off.
///
/// `host_override` (`[remote] host`) replaces the auto-discovered LAN IP in the connect
/// URL/QR/cert; only meaningful for the LAN exposure.
pub(crate) async fn toggle_remote(
    remote: &mut Option<remote::RemoteControl>,
    app: &mut forge_tui::App,
    _tui: &mut forge_tui::Tui,
    exposure: remote::Exposure,
    remote_cfg: &forge_config::RemoteConfig,
    history: remote::HistoryProvider,
) -> Result<()> {
    if let Some(rc) = remote.take() {
        // Turning it off: the handle's Drop aborts the server task + tunnel and sends a `closed`
        // snapshot so any connected browser stops reconnecting.
        app.remote_active = false;
        app.note("◉ remote control off — browser disconnected");
        drop(rc);
        return Ok(());
    }
    let anywhere = exposure == remote::Exposure::Anywhere;
    if anywhere {
        app.note("◉ remote control — opening a public tunnel (this can take a few seconds)…");
    }
    let started = match exposure {
        remote::Exposure::Anywhere => remote::start_anywhere(Some(history), remote_cfg).await,
        other => remote::start(other, remote_cfg.host.as_deref(), Some(history)),
    };
    match started {
        Ok(rc) => {
            app.remote_active = true;
            let where_ = match exposure {
                remote::Exposure::Lan => "LAN".to_string(),
                remote::Exposure::Local => "loopback".to_string(),
                remote::Exposure::Anywhere => {
                    format!("public tunnel via {}", rc.tunnel.unwrap_or("tunnel"))
                }
            };
            app.note(&format!(
                "◉ remote control on — listening on {} ({where_})",
                rc.url.addr,
            ));
            if anywhere {
                // A public URL is reachable from the whole internet; the path token is the only
                // gate. Make that explicit so the user knows what they've opened.
                app.note(
                    "  ⚠ anyone with the link can drive this session — the token is the only gate",
                );
            }
            app.note(&format!("  connect: {}", rc.url.url));
            if let Some(qr) = remote::qr_lines(&rc.url.url) {
                app.print_lines(qr);
            }
            *remote = Some(rc);
        }
        Err(e) => {
            app.note(&format!("⚠ could not start remote control: {e}"));
        }
    }
    Ok(())
}

/// First use of a *project*-scope command/skill is confirmed by re-running it (its name is
/// "armed" on the first attempt and runs on the second) — unless project scope is trusted. User-
/// scope and builtins are never gated. Returns true when the invocation may proceed.
pub(crate) fn project_trust_ok(
    name: &str,
    scope: forge_skills::Scope,
    trust_project: bool,
    armed: &mut std::collections::HashSet<String>,
    app: &mut forge_tui::App,
) -> bool {
    if scope != forge_skills::Scope::Project || trust_project || armed.contains(name) {
        return true;
    }
    armed.insert(name.to_string());
    app.note(&format!(
        "⚠ /{name} is a project command — it can steer the model. Run it again to confirm."
    ));
    false
}

/// Populate + open the session picker from the store (newest first). `query` pre-fills the filter.
/// A clean, single-line title for a session row, derived from its first user prompt: newlines and
/// runs of whitespace collapse to single spaces, leading `/command` noise is kept, and the result
/// is trimmed to a readable length. Falls back to a placeholder when the session has no prompt.
pub(crate) fn session_title(preview: Option<&str>) -> String {
    let raw = preview.unwrap_or("").trim();
    if raw.is_empty() {
        return "(no prompt yet)".to_string();
    }
    let collapsed: String = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let max = 64;
    if collapsed.chars().count() > max {
        format!("{}…", collapsed.chars().take(max - 1).collect::<String>())
    } else {
        collapsed
    }
}

/// Surface what an undo/restore did to the user's files.
pub(crate) fn note_restore(app: &mut forge_tui::App, report: &forge_core::snapshot::RestoreReport) {
    if !report.restored.is_empty() {
        app.note(&format!("↺ restored {} file(s)", report.restored.len()));
    }
    for w in &report.warnings {
        app.note(&format!(
            "⚠ {w} changed since Forge wrote it — overwrote your edit"
        ));
    }
    for f in &report.failed {
        app.note(&format!("✗ failed to restore {f}"));
    }
}

/// A short relative age like "3m ago" / "2h ago" / "5d ago" from an epoch-second timestamp.
pub(crate) fn fmt_age(created_at: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let secs = (now - created_at).max(0);
    if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86_400)
    }
}

fn find_starting_event_id(store: &forge_store::Store, session_id: &str) -> i64 {
    if let Ok(events) = store.live_events_after(session_id, 0) {
        for (id, json) in events.iter().rev() {
            if let Ok(ev) = serde_json::from_str::<crate::live_observer::LiveEvent>(json) {
                if matches!(ev, crate::live_observer::LiveEvent::AssistantDone) {
                    return *id;
                }
            }
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picker_rows(ids: &[&str]) -> Vec<forge_tui::PickerRow> {
        ids.iter()
            .map(|id| forge_tui::PickerRow {
                id: id.to_string(),
                title: id.to_string(),
                subtitle: String::new(),
            })
            .collect()
    }

    #[test]
    fn voice_ptt_tap_vs_hold_threshold() {
        // Faster than the threshold: a tap — leave the overlay in ordinary toggle mode.
        assert!(!voice_is_hold(0));
        assert!(!voice_is_hold(VOICE_PTT_HOLD_MS - 1));
        // At or past the threshold: a hold — auto-stop + transcribe on release.
        assert!(voice_is_hold(VOICE_PTT_HOLD_MS));
        assert!(voice_is_hold(VOICE_PTT_HOLD_MS + 1));
        assert!(voice_is_hold(5_000));
    }

    #[test]
    fn one_shot_slash_passthrough_and_escape() {
        // A plain prompt is untouched.
        let (p, g, t) = expand_one_shot_slash("fix the bug").unwrap();
        assert_eq!(p, "fix the bug");
        assert!(g.is_empty());
        assert!(t.is_none());
        // `//foo` escapes to a literal `/foo` prompt (mirrors chat).
        let (p, _, _) = expand_one_shot_slash("//rust is neat").unwrap();
        assert_eq!(p, "/rust is neat");
        // An absolute path is NOT treated as a command — it must pass through verbatim.
        let (p, g, _) = expand_one_shot_slash("/no/such/command-path explain this").unwrap();
        assert_eq!(p, "/no/such/command-path explain this");
        assert!(g.is_empty());
    }

    #[test]
    fn overlay_ops_are_noops_when_nothing_modal_is_open() {
        let mut app = forge_tui::App::default();
        // Cancel/Nav/Select must NEVER synthesize keys into an overlay-free session — a stray
        // Esc would interrupt a busy turn or quit an idle host.
        assert!(apply_overlay_input(&mut app, RemoteOverlayOp::Cancel).is_empty());
        assert!(apply_overlay_input(&mut app, RemoteOverlayOp::Nav(3)).is_empty());
        assert!(apply_overlay_input(&mut app, RemoteOverlayOp::Select("x".into())).is_empty());
    }

    #[test]
    fn overlay_nav_and_cancel_map_to_arrow_and_esc_keys() {
        use forge_tui::KeyKind as K;
        let mut app = forge_tui::App::default();
        app.picker.open_with(
            forge_tui::PickerKind::Sessions,
            "resume",
            picker_rows(&["a", "b", "c"]),
        );
        assert_eq!(
            apply_overlay_input(&mut app, RemoteOverlayOp::Nav(2)),
            vec![K::Down, K::Down]
        );
        assert_eq!(
            apply_overlay_input(&mut app, RemoteOverlayOp::Nav(-1)),
            vec![K::Up]
        );
        assert_eq!(
            apply_overlay_input(&mut app, RemoteOverlayOp::Nav(0)),
            Vec::<K>::new()
        );
        // Bounded against a hostile delta.
        assert_eq!(
            apply_overlay_input(&mut app, RemoteOverlayOp::Nav(i32::MIN)).len(),
            100
        );
        assert_eq!(
            apply_overlay_input(&mut app, RemoteOverlayOp::Cancel),
            vec![K::Esc]
        );
    }

    #[test]
    fn overlay_select_moves_the_picker_cursor_then_commits_with_enter() {
        use forge_tui::KeyKind as K;
        let mut app = forge_tui::App::default();
        app.picker.open_with(
            forge_tui::PickerKind::Tempers,
            "switch operating mode",
            picker_rows(&["Survey", "Guarded", "Smith"]),
        );
        let keys = apply_overlay_input(&mut app, RemoteOverlayOp::Select("Smith".into()));
        assert_eq!(keys, vec![K::Enter], "select = cursor move + Enter");
        assert_eq!(app.picker.selected_row().unwrap().id, "Smith");
        // An id that isn't in the (filtered) rows commits nothing.
        assert!(apply_overlay_input(&mut app, RemoteOverlayOp::Select("nope".into())).is_empty());
    }

    #[test]
    fn overlay_filter_narrows_the_picker_and_select_respects_it() {
        let mut app = forge_tui::App::default();
        app.picker.open_with(
            forge_tui::PickerKind::Sessions,
            "resume",
            picker_rows(&["alpha", "beta"]),
        );
        assert!(apply_overlay_input(&mut app, RemoteOverlayOp::Filter("bet".into())).is_empty());
        assert_eq!(app.picker.query, "bet");
        assert_eq!(app.picker.matches().len(), 1);
        // "alpha" is filtered out → selecting it is a no-op, not a mis-commit of "beta".
        assert!(apply_overlay_input(&mut app, RemoteOverlayOp::Select("alpha".into())).is_empty());
    }

    #[test]
    fn overlay_select_on_the_palette_materializes_the_command_line() {
        use forge_tui::KeyKind as K;
        let mut app = forge_tui::App::default();
        app.palette.open_with("");
        let keys = apply_overlay_input(&mut app, RemoteOverlayOp::Select("model".into()));
        assert_eq!(keys, vec![K::Enter]);
        // The palette's Enter dispatches `app.input` when it starts with '/' — the select must
        // have staged exactly what a local user would have typed.
        assert_eq!(app.input, "/model");
        assert_eq!(app.input_cursor, app.input.len());
    }

    #[test]
    fn overlay_filter_writes_the_config_edit_buffer_while_editing() {
        let mut app = forge_tui::App::default();
        app.config_editor.open_with(vec![forge_tui::SettingRow {
            path: "tui.fullscreen".into(),
            label: "Full-screen".into(),
            group: "tui".into(),
            value: "true".into(),
            ..Default::default()
        }]);
        // Not editing: filter narrows the row list.
        apply_overlay_input(&mut app, RemoteOverlayOp::Filter("full".into()));
        assert_eq!(app.config_editor.filter, "full");
        // Editing: the same verb replaces the pending VALUE (the page's free-text box), which
        // the synthesized Enter then commits through ConfigEditor::handle_key.
        app.config_editor.editing = Some(String::new());
        apply_overlay_input(&mut app, RemoteOverlayOp::Filter("false".into()));
        assert_eq!(app.config_editor.editing.as_deref(), Some("false"));
        assert_eq!(app.config_editor.filter, "full", "filter untouched");
    }

    #[test]
    fn overlay_select_on_informational_overlays_moves_the_cursor_only() {
        let mut app = forge_tui::App::default();
        app.mesh_overlay.open = true;
        app.mesh_overlay.candidates = vec![
            forge_tui::MeshCandRow {
                model: "a".into(),
                ..Default::default()
            },
            forge_tui::MeshCandRow {
                model: "b".into(),
                ..Default::default()
            },
        ];
        let keys = apply_overlay_input(&mut app, RemoteOverlayOp::Select("b".into()));
        assert!(
            keys.is_empty(),
            "mesh rows are a browsing highlight, no Enter"
        );
        assert_eq!(app.mesh_overlay.cursor, 1);
    }

    /// The e2e-style drive of the parity mechanism: a remote client opens the `/model` picker,
    /// navigates, and selects — and the session's model pin actually changes. Uses the REAL
    /// pieces of the path: `apply_overlay_input` for the wire verbs, the picker's own
    /// `move_up`/`move_down` (exactly what the key loop calls for ↑/↓), and `picker_accept`
    /// (exactly what the key loop calls on Enter for `ModelPin`).
    #[tokio::test]
    async fn remote_drive_of_the_model_pin_picker_changes_the_pin() {
        use forge_tui::KeyKind as K;
        let config = forge_config::Config::default();
        let session = Arc::new(tokio::sync::Mutex::new(
            Session::start(
                Arc::new(forge_store::Store::open_in_memory().unwrap()),
                Arc::new(forge_provider::MockProvider),
                Arc::new(forge_mesh::HeuristicRouter::new(config.clone())),
                ToolRegistry::with_core_tools(),
                Box::new(forge_tui::HeadlessPresenter::new(false)),
                config,
                ".",
            )
            .unwrap(),
        ));
        assert_eq!(session.lock().await.pinned_model(), None);

        let mut app = forge_tui::App::default();
        app.picker.open_with(
            forge_tui::PickerKind::ModelPin,
            "⊕ pin model",
            picker_rows(&["mesh", "groq::llama-3.3-70b", "groq::qwen3-32b"]),
        );
        // The phone shows the projected overlay…
        let overlay = app.remote_overlay().expect("picker projects");
        assert_eq!(overlay.kind, "picker:model_pin");
        assert_eq!(overlay.rows.len(), 3);

        // …navigates down twice (OverlayNav{delta:2} → two Down keys through the key path)…
        for key in apply_overlay_input(&mut app, RemoteOverlayOp::Nav(2)) {
            match key {
                K::Down => app.picker.move_down(),
                K::Up => app.picker.move_up(),
                other => panic!("nav synthesizes only arrows, got {other:?}"),
            }
        }
        assert_eq!(app.picker.selected_row().unwrap().id, "groq::qwen3-32b");

        // …then taps the llama row (OverlaySelect → cursor move + synthesized Enter).
        let keys = apply_overlay_input(
            &mut app,
            RemoteOverlayOp::Select("groq::llama-3.3-70b".into()),
        );
        assert_eq!(keys, vec![K::Enter]);
        let row = app.picker.selected_row().cloned().expect("row selected");
        let kind = app.picker.kind.expect("picker kind");
        app.picker.close();
        picker_accept(kind, &row, &session, None, &mut app)
            .await
            .unwrap();
        assert_eq!(
            session.lock().await.pinned_model(),
            Some("groq::llama-3.3-70b"),
            "the remote pick pinned the model"
        );

        // And picking "mesh" the same way clears the pin again.
        app.picker.open_with(
            forge_tui::PickerKind::ModelPin,
            "⊕ pin model",
            picker_rows(&["mesh", "groq::llama-3.3-70b"]),
        );
        let keys = apply_overlay_input(&mut app, RemoteOverlayOp::Select("mesh".into()));
        assert_eq!(keys, vec![K::Enter]);
        let row = app.picker.selected_row().cloned().unwrap();
        let kind = app.picker.kind.unwrap();
        app.picker.close();
        picker_accept(kind, &row, &session, None, &mut app)
            .await
            .unwrap();
        assert_eq!(session.lock().await.pinned_model(), None, "pin cleared");
    }

    /// The actual race being fixed: an `Attach` for an unrelated image lands first (ambient
    /// `pending_images`, e.g. from an adjacent message's upload), then a `Prompt` arrives with
    /// its OWN explicit `attachments` list — the resulting turn must carry ONLY what that list
    /// specified, never the stale ambient one.
    #[tokio::test]
    async fn explicit_prompt_attachments_discard_stale_ambient_pending_images() {
        let dir = std::env::temp_dir().join(format!(
            "forge-prompt-attach-race-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let uploads = dir.join(".forge").join("uploads");
        std::fs::create_dir_all(&uploads).unwrap();
        let cwd = dir.display().to_string();

        let config = forge_config::Config::default();
        let session = Arc::new(tokio::sync::Mutex::new(
            Session::start(
                Arc::new(forge_store::Store::open_in_memory().unwrap()),
                Arc::new(forge_provider::MockProvider),
                Arc::new(forge_mesh::HeuristicRouter::new(config.clone())),
                ToolRegistry::with_core_tools(),
                Box::new(forge_tui::HeadlessPresenter::new(false)),
                config,
                ".",
            )
            .unwrap(),
        ));

        // Simulate the stale ambient `Attach`: an image meant for a DIFFERENT, adjacent
        // message, already sitting in the session's pending-images queue.
        session
            .lock()
            .await
            .attach_images(vec![forge_types::ImageAttachment {
                media_type: "image/jpeg".into(),
                data_base64: "stale-unrelated-image".into(),
            }]);

        let mut app = forge_tui::App::default();
        let mut remote_notes = Vec::new();

        // This message's OWN correlated attachment: a real image inside the upload area.
        let real_path = uploads.join("real.png");
        std::fs::write(&real_path, b"not-real-png-bytes-but-thats-fine-here").unwrap();
        let attachments = vec![remote::PromptAttachment {
            path: real_path.display().to_string(),
            image: true,
        }];

        let mentions =
            resolve_prompt_attachments(&session, &mut app, &mut remote_notes, &cwd, attachments)
                .await;
        assert!(mentions.is_empty(), "no non-image attachments in this list");

        let images = session.lock().await.take_pending_images();
        assert_eq!(
            images.len(),
            1,
            "only the explicit list's image rides this turn"
        );
        assert_eq!(images[0].media_type, "image/png");
        assert_ne!(
            images[0].data_base64, "stale-unrelated-image",
            "the stale ambient image from the unrelated Attach must not leak in"
        );

        // An empty attachments list (the fallback case) must leave ambient state untouched —
        // it's a no-op, not a second discard.
        session
            .lock()
            .await
            .attach_images(vec![forge_types::ImageAttachment {
                media_type: "image/gif".into(),
                data_base64: "still-ambient".into(),
            }]);
        let mentions =
            resolve_prompt_attachments(&session, &mut app, &mut remote_notes, &cwd, Vec::new())
                .await;
        assert!(mentions.is_empty());
        let images = session.lock().await.take_pending_images();
        assert_eq!(
            images.len(),
            1,
            "empty list leaves ambient pending_images alone"
        );
        assert_eq!(images[0].data_base64, "still-ambient");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A path outside `.forge/uploads/` in an explicit attachments list is refused with a note,
    /// exactly like the ambient `Attach` confinement check — a WS client can't use the new
    /// correlated path to read an arbitrary host file either.
    #[tokio::test]
    async fn explicit_prompt_attachments_confines_paths_to_the_upload_area() {
        let dir = std::env::temp_dir().join(format!(
            "forge-prompt-attach-confine-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join(".forge").join("uploads")).unwrap();
        let outside = dir.join("outside.txt");
        std::fs::write(&outside, b"not an upload").unwrap();
        let cwd = dir.display().to_string();

        let config = forge_config::Config::default();
        let session = Arc::new(tokio::sync::Mutex::new(
            Session::start(
                Arc::new(forge_store::Store::open_in_memory().unwrap()),
                Arc::new(forge_provider::MockProvider),
                Arc::new(forge_mesh::HeuristicRouter::new(config.clone())),
                ToolRegistry::with_core_tools(),
                Box::new(forge_tui::HeadlessPresenter::new(false)),
                config,
                ".",
            )
            .unwrap(),
        ));
        let mut app = forge_tui::App::default();
        let mut remote_notes = Vec::new();

        let mentions = resolve_prompt_attachments(
            &session,
            &mut app,
            &mut remote_notes,
            &cwd,
            vec![remote::PromptAttachment {
                path: outside.display().to_string(),
                image: false,
            }],
        )
        .await;
        assert!(mentions.is_empty(), "refused path never becomes a mention");
        assert!(remote_notes
            .iter()
            .any(|n| n.contains("not a file from this session's upload area")));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn known_provider_prefixes_accept_real_providers_and_keyless_bridges() {
        assert!(is_known_provider_prefix("groq"));
        assert!(is_known_provider_prefix("anthropic"));
        assert!(is_known_provider_prefix("ollama"));
        assert!(is_known_provider_prefix("claude-cli"));
        assert!(is_known_provider_prefix("codex-cli"));
        // Clearly-invalid prefixes are rejected so `--model` hard-stops without a catalog.
        assert!(!is_known_provider_prefix("nonsense"));
        assert!(!is_known_provider_prefix("gpt-5"));
    }

    #[test]
    fn goal_completion_marker_must_be_a_standalone_reply() {
        assert!(is_goal_complete_marker(Some("GOAL COMPLETE")));
        assert!(is_goal_complete_marker(Some("\n  GOAL COMPLETE\n")));
        assert!(!is_goal_complete_marker(Some("Goal complete — fixed it.")));
        assert!(!is_goal_complete_marker(Some(
            "I will reply GOAL COMPLETE when done."
        )));
        assert!(!is_goal_complete_marker(None));
    }

    #[test]
    fn goal_stop_reason_stops_when_model_says_complete() {
        assert_eq!(
            goal_stop_reason(true, 1, 3, 2, 0),
            Some(GOAL_COMPLETE_REASON)
        );
    }

    #[test]
    fn goal_stop_reason_stops_when_all_tasks_done() {
        assert_eq!(
            goal_stop_reason(false, 3, 3, 2, 0),
            Some("🎯 goal complete — all tasks done")
        );
    }

    #[test]
    fn goal_stop_reason_stops_at_iteration_ceiling() {
        assert_eq!(
            goal_stop_reason(false, 1, 3, GOAL_MAX_ITERS, 0),
            Some("🎯 goal stopped — iteration ceiling")
        );
    }

    #[test]
    fn goal_stop_reason_stops_when_wedged() {
        assert_eq!(
            goal_stop_reason(false, 1, 3, 2, GOAL_NO_PROGRESS_MAX),
            Some("🎯 goal stalled — no task progress, stopping")
        );
    }

    #[test]
    fn goal_stop_reason_continues_with_open_tasks_and_progress() {
        assert_eq!(goal_stop_reason(false, 1, 3, 2, 0), None);
    }
}
