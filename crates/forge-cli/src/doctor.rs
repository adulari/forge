//! `forge doctor` — diagnose a user's environment in one command: config, providers/keys, CLI
//! bridges, the local Ollama runtime, git, and the terminal — each with an actionable fix. The
//! single biggest lever for onboarding + support (and the first thing to paste into a bug report).
//!
//! Doctor tests *function*, not just *presence*: a key being set, a binary being on PATH, or a
//! port being open does NOT mean a turn can run. So beyond the local/static checks it does two
//! bounded LIVE probes — a keyed provider's `list_models` (free) and a CLI-bridge round-trip
//! ($0 on a subscription) — each behind a timeout. These catch the real "doctor says fine but
//! Forge is unusable" cases: a keyed provider that's unreachable (→ keyless fallback churn) and a
//! bridge that's on PATH but can't actually launch (the Windows `cmd /S /C` shim path).

#[path = "doctor_bridge_models.rs"]
mod bridge_models;

use crate::local;

/// One diagnostic line's outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Status {
    Ok,
    Warn,
    Fail,
    Info,
}

impl Status {
    fn glyph(self) -> &'static str {
        match self {
            Status::Ok => "✓",
            Status::Warn => "⚠",
            Status::Fail => "✗",
            Status::Info => "·",
        }
    }
}

pub(crate) struct Check {
    pub(crate) status: Status,
    pub(crate) label: String,
    pub(crate) detail: String,
    /// An actionable next step, shown when not `Ok`.
    pub(crate) fix: Option<String>,
}

impl Check {
    fn print(&self) {
        println!(
            "  {} {:<22} {}",
            self.status.glyph(),
            self.label,
            self.detail
        );
        if self.status != Status::Ok && self.status != Status::Info {
            if let Some(fix) = &self.fix {
                println!("      → {fix}");
            }
        }
    }
}

pub(crate) fn check(
    status: Status,
    label: &str,
    detail: impl Into<String>,
    fix: Option<&str>,
) -> Check {
    Check {
        status,
        label: label.to_string(),
        detail: detail.into(),
        fix: fix.map(str::to_string),
    }
}

/// Run all diagnostics and print a report. Returns the number of hard failures (for the exit code).
pub async fn run() -> anyhow::Result<usize> {
    println!("⚒ forge doctor — {}\n", env!("CARGO_PKG_VERSION"));

    let mut sections: Vec<(&str, Vec<Check>)> = Vec::new();
    sections.push(("Config", config_checks()));
    let (mut provider_v, has_usable_provider) = provider_checks();
    // Reachability is evidence for the routing verdict, not the verdict itself. A provider can
    // answer discovery while the mesh has excluded it or exhausted its subscription.
    let reachability = provider_reachability_checks().await;
    // OpenCode Go's windows are poll-only, so without this the routing verdict below would report
    // whatever a previous command happened to leave behind (or nothing at all).
    if let Ok(store) = crate::open_store() {
        crate::cli::commands::models::refresh_opencode_go_quota(&store).await;
    }
    let routing = crate::doctor_health::provider_routing_checks(&reachability);
    let has_usable_provider = if routing.is_empty() {
        has_usable_provider
    } else {
        routing.iter().any(|check| check.status == Status::Ok)
            || local::ollama_installed() && !local::ollama_installed_models().is_empty()
    };
    provider_v.extend(routing);
    provider_v.extend(reachability);
    sections.push(("Providers", provider_v));
    // Live, timeout-bounded: prove a detected bridge can actually launch + answer (not just on PATH).
    let bridge_live = bridge_roundtrip_checks().await;
    if !bridge_live.is_empty() {
        sections.push(("Bridge liveness", bridge_live));
    }
    let bridge_models = bridge_models::checks().await;
    if !bridge_models.is_empty() {
        sections.push(("Bridge model discovery", bridge_models));
    }
    sections.push(("Model health", crate::doctor_health::model_health_checks()));
    sections.push(("Background daemon", daemon_checks().await));
    sections.push(("Forge Anywhere", crate::doctor_health::anywhere_checks()));
    sections.push(("Local LLM (Ollama)", ollama_checks()));
    sections.push(("Session store", store_checks()));
    // Worktree build artifacts are the quietest way to lose a machine: 76 registered worktrees
    // filled a 1.8 TB disk to zero free and killed a linker with a bus error before anything
    // reported the cost. Surveying is git + a directory walk, so it is skipped outside a repo.
    if let Some(worktrees) = crate::cli::commands::worktree::doctor_summary() {
        sections.push(("Worktree disk", vec![worktrees]));
    }
    sections.push((
        "Environment",
        crate::doctor_environment::environment_checks(),
    ));

    let mut fails = 0;
    let mut warns = 0;
    for (title, checks) in &sections {
        println!("{title}");
        for c in checks {
            c.print();
            match c.status {
                Status::Fail => fails += 1,
                Status::Warn => warns += 1,
                _ => {}
            }
        }
        println!();
    }

    // The one gate that actually blocks usage: a routable provider must exist.
    if !has_usable_provider {
        fails += 1;
        println!("✗ No usable model provider configured — Forge can't route a turn.");
        println!(
            "  Run `forge setup` (add an API key, a CLI-bridge subscription, or a local model).\n"
        );
    }

    if fails == 0 && warns == 0 {
        println!("All good — Forge is ready. ⚒");
    } else {
        println!(
            "{fails} failure(s), {warns} warning(s). Address the ✗ items above; ⚠ are optional.",
        );
    }
    Ok(fails)
}

/// Can this binary actually open the session store?
///
/// A store migrated by a newer build is the failure this exists for. It crash-looped
/// `forge-serve.service` and, more insidiously, took the Anywhere connector down while local Forge
/// kept answering on a connection opened before the migration — so "the daemon is running" looked
/// healthy while cloud sync was dead. Nothing in doctor asked the one question that explains it.
/// The store this process would actually open.
///
/// `FORGE_DB` overrides the default path everywhere else (see `replay.rs`, the bridge env
/// passthrough in `cli_provider.rs`, and every test that isolates its store). Doctor ignored it and
/// always reported on `data_dir()/forge.db`, so it could answer "the session store opens cleanly"
/// about a DIFFERENT database than the one the session under investigation uses — which is exactly
/// backwards for the tool people run when the store is the suspect.
pub(crate) fn doctor_store_path() -> Option<std::path::PathBuf> {
    resolve_store_path(
        std::env::var("FORGE_DB").ok().as_deref(),
        forge_config::data_dir(),
    )
}

/// Pure so the override is testable without mutating process env, which races under the parallel
/// test harness. A blank `FORGE_DB` counts as unset rather than as a request to open "".
fn resolve_store_path(
    forge_db: Option<&str>,
    data_dir: Option<std::path::PathBuf>,
) -> Option<std::path::PathBuf> {
    if let Some(custom) = forge_db.map(str::trim).filter(|s| !s.is_empty()) {
        return Some(std::path::PathBuf::from(custom));
    }
    data_dir.map(|dir| dir.join("forge.db"))
}

fn store_checks() -> Vec<Check> {
    let Some(path) = doctor_store_path() else {
        return vec![check(
            Status::Warn,
            "session store",
            "no data directory resolves on this platform",
            None,
        )];
    };
    vec![store_check_at(&path), price_feed_check_at(&path)]
}

/// How current the fetched price feed is. Cost-aware routing derives subscription burn weights
/// from `model_pricing`; a feed that stopped refreshing is invisible everywhere else and its
/// symptom — a heavy model winning a shared pool at zero penalty — looks like a routing bug.
/// Live case: a schema hole failed every price upsert from 2026-06-18 to 2026-09-02.
fn price_feed_check_at(path: &std::path::Path) -> Check {
    const STALE_AFTER_SECS: i64 = 7 * 24 * 60 * 60;
    let Ok(store) = forge_store::Store::open(path) else {
        return check(Status::Warn, "price feed", "store not readable", None);
    };
    match store.model_pricing_freshness() {
        Ok((rows, Some(newest))) => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() as i64);
            let age = now - newest;
            let age_text = if age < 3600 {
                format!("{}m ago", age / 60)
            } else if age < 86_400 {
                format!("{}h ago", age / 3600)
            } else {
                format!("{}d ago", age / 86_400)
            };
            if age > STALE_AFTER_SECS {
                check(
                    Status::Warn,
                    "price feed",
                    format!("{rows} model price(s), last refreshed {age_text} — stale"),
                    Some(
                        "run `forge models` to refresh; if it stays stale, check `forge models` \
                         output for a 'could not persist a fetched model price' warning",
                    ),
                )
            } else {
                check(
                    Status::Ok,
                    "price feed",
                    format!("{rows} model price(s), refreshed {age_text}"),
                    None,
                )
            }
        }
        Ok((_, None)) => check(
            Status::Warn,
            "price feed",
            "no model prices fetched yet",
            Some("run `forge models` once online so cost-aware routing has real prices"),
        ),
        Err(error) => check(
            Status::Warn,
            "price feed",
            format!("could not read model_pricing: {error}"),
            None,
        ),
    }
}

fn store_check_at(path: &std::path::Path) -> Check {
    if !path.exists() {
        return check(
            Status::Ok,
            "session store",
            format!("not created yet — {}", path.display()),
            None,
        );
    }
    match forge_store::Store::open(path) {
        Ok(_) => check(
            Status::Ok,
            "session store",
            format!("opens cleanly — {}", path.display()),
            None,
        ),
        Err(error) => {
            let detail = error.to_string();
            // `Store::open` already reports both versions in this case; the useful addition is
            // saying what to do about it, which otherwise takes a journal hunt to work out.
            let fix = if detail.contains("newer than this build supports") {
                Some(
                    "a newer Forge migrated this store — install the current release \
                     (`forge update`), or point this build elsewhere with FORGE_DB",
                )
            } else {
                Some("check the file is readable and not held by a broken process")
            };
            check(Status::Fail, "session store", detail, fix)
        }
    }
}

fn config_checks() -> Vec<Check> {
    let mut out = Vec::new();
    match forge_config::load() {
        Ok(_) => out.push(check(Status::Ok, "config", "loads cleanly", None)),
        Err(e) => out.push(check(
            Status::Fail,
            "config",
            format!("failed to load: {e}"),
            Some("fix the syntax in your config.toml (see `forge doctor` detail above)"),
        )),
    }
    // Which store the key checks below read. A `forge` bin built by `cargo test` carries the
    // in-memory `test-secrets` store, so "no keys" from it says nothing about the user's setup.
    let backend = forge_config::secret_store::backend();
    out.push(check(
        if backend.is_blind() {
            Status::Warn
        } else {
            Status::Ok
        },
        "secret store",
        backend.describe(),
        backend.is_blind().then_some(
            "rebuild with `cargo build --bin forge` — this binary cannot see any stored key",
        ),
    ));
    let user = forge_config::config_dir().map(|d| d.join("config.toml"));
    let user_exists = user.as_ref().is_some_and(|p| p.exists());
    out.push(check(
        if user_exists {
            Status::Ok
        } else {
            Status::Info
        },
        "user config",
        match &user {
            Some(p) if user_exists => p.display().to_string(),
            Some(p) => format!("{} (not created yet)", p.display()),
            None => "no config dir resolved".to_string(),
        },
        None,
    ));
    if std::path::Path::new("./.forge/config.toml").exists() {
        out.push(check(
            Status::Info,
            "project config",
            "./.forge/config.toml",
            None,
        ));
    }
    // Data dir writable (the session store lives here).
    match forge_config::data_dir() {
        Some(d) => {
            let writable = std::fs::create_dir_all(&d).is_ok();
            out.push(check(
                if writable { Status::Ok } else { Status::Fail },
                "data dir",
                d.display().to_string(),
                (!writable).then_some("ensure the data directory is writable"),
            ));
        }
        None => out.push(check(
            Status::Warn,
            "data dir",
            "could not resolve a data directory",
            Some("set $XDG_DATA_HOME or $HOME"),
        )),
    }
    out
}

/// Provider checks + whether at least one routable provider exists.
fn provider_checks() -> (Vec<Check>, bool) {
    let mut out = Vec::new();
    let mut usable = false;

    // API keys (env or keyring).
    let mut any_key = false;
    for p in forge_config::known_key_providers() {
        if forge_config::has_api_key(p) {
            any_key = true;
            usable = true;
            out.push(check(Status::Ok, &format!("{p} key"), "configured", None));
        }
    }
    if !any_key {
        out.push(check(
            Status::Info,
            "API keys",
            "none configured",
            Some("`forge auth <provider>` or `/config` to add one (optional if you use bridges/local)"),
        ));
    }

    // Subscription CLI bridges.
    for k in forge_provider::CliKind::all() {
        let avail = k.available();
        if avail {
            usable = true;
        }
        out.push(check(
            if avail { Status::Ok } else { Status::Info },
            &format!("{} bridge", k.prefix()),
            if avail { "installed" } else { "not installed" },
            (!avail).then_some(match k {
                forge_provider::CliKind::ClaudeCode => {
                    "install Claude Code + run `claude` once to log in (optional)"
                }
                forge_provider::CliKind::Codex => "install Codex + run `codex login` (optional)",
                forge_provider::CliKind::Antigravity => {
                    "install Antigravity + run `agy` once to log in (optional)"
                }
            }),
        ));
    }

    // A local model counts as a usable provider too.
    if local::ollama_installed() && !local::ollama_installed_models().is_empty() {
        usable = true;
    }
    (out, usable)
}

fn ollama_checks() -> Vec<Check> {
    let mut out = Vec::new();
    match local::ollama_version() {
        Some(v) => out.push(check(Status::Ok, "ollama", v, None)),
        None => {
            out.push(check(
                Status::Info,
                "ollama",
                "not installed",
                Some("`forge local install` to run models locally (optional)"),
            ));
            return out;
        }
    }
    out.push(check(
        if local::ollama_serving() {
            Status::Ok
        } else {
            Status::Info
        },
        "server",
        if local::ollama_serving() {
            "running (localhost:11434)"
        } else {
            "stopped"
        },
        (!local::ollama_serving()).then_some("`forge local start` to run the server + model"),
    ));
    let models = local::ollama_installed_models();
    out.push(check(
        if models.is_empty() {
            Status::Info
        } else {
            Status::Ok
        },
        "models",
        if models.is_empty() {
            "none pulled".to_string()
        } else {
            models.join(", ")
        },
        models
            .is_empty()
            .then_some("`forge local install` to pull a model"),
    ));
    out
}

/// Truncate a provider/error string to one tidy line for the report.
pub(crate) fn short(s: &str) -> String {
    let line = s.lines().next().unwrap_or("").trim();
    if line.chars().count() > 90 {
        format!("{}…", line.chars().take(89).collect::<String>())
    } else {
        line.to_string()
    }
}

/// The background daemon (`forge service`). Doctor previously said nothing about it, so a machine
/// whose daemon was dead — Anywhere unreachable, the phone unable to see any session — reported a
/// clean bill of health. That is exactly what happened on 2026-08-07: `forge-serve` sat at 632
/// consecutive failed restarts against a store it could not open, and the only evidence anywhere
/// was one journal line.
///
/// Not installed is `Info`, not a failure: the daemon is opt-in and plenty of people never run it.
/// What matters is an installed daemon that is NOT serving, and the state word distinguishes the
/// cases that need different fixes (`failed` vs a restart loop vs simply stopped).
async fn daemon_checks() -> Vec<Check> {
    let status = match crate::cli::commands::service::query_service_status() {
        Ok(s) => s,
        // A missing/unavailable service manager is not a Forge fault — report and move on.
        Err(e) => {
            return vec![check(
                Status::Info,
                "background daemon",
                format!(
                    "could not query the service manager: {}",
                    short(&e.to_string())
                ),
                None,
            )]
        }
    };
    if !status.installed {
        return vec![check(
            Status::Info,
            "background daemon",
            "not installed",
            Some(
                "`forge service install` to run sessions in the background and reach them remotely",
            ),
        )];
    }

    let mut out = vec![daemon_state_check(status.detail.trim())];

    // Installed and claiming to run is not the same as serving: a daemon that is up but whose
    // listener never bound is invisible to every client, which is the failure a state word misses.
    let port = crate::cli::commands::service::resolved_port(None);
    let responding = crate::cli::commands::service::probe_port(port);
    out.push(match (status.running, responding) {
        (true, true) => check(Status::Ok, "daemon port", format!("{port} responding"), None),
        (true, false) => check(
            Status::Fail,
            "daemon port",
            format!("{port} not responding while the service reports running"),
            Some("`forge service restart`; if it persists, check the port is not taken by another process"),
        ),
        // Not running — the state check above already reported why; don't double-count it.
        (false, _) => check(
            Status::Info,
            "daemon port",
            format!("{port} not responding (daemon not running)"),
            None,
        ),
    });

    // A listening socket alone does not prove the daemon can serve its fleet.  Exercise the
    // authenticated discovery endpoint as a cheap end-to-end check; this catches stale listeners
    // and routers that are alive while their backing store is unusable.
    if status.running && responding {
        out.push(crate::doctor_health::daemon_fleet_check(port).await);
    }

    #[cfg(target_os = "linux")]
    if let Some(unit) = crate::cli::commands::service::installed_unit_text() {
        let exe = std::env::current_exe()
            .ok()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        out.extend(crate::doctor_daemon::unit_drift_checks(&unit, &exe, port).await);
    }
    out
}

/// State word from the service manager → the line doctor prints. Pure so every branch is testable
/// without installing, starting or breaking a real service on the host running the tests.
fn daemon_state_check(state: &str) -> Check {
    match state {
        "active" => check(Status::Ok, "background daemon", "running", None),
        // The silent killer: systemd reports `activating` while a unit restart-loops, so nothing
        // ever looks broken even after hundreds of consecutive failures.
        "activating" | "auto-restart" => check(
            Status::Fail,
            "background daemon",
            format!("{state} — restarting repeatedly, not serving"),
            Some("`journalctl --user -u forge-serve.service -n 30` for the cause; a schema mismatch means the installed binary is older than the store"),
        ),
        "failed" => check(
            Status::Fail,
            "background daemon",
            "failed",
            Some("`journalctl --user -u forge-serve.service -n 30` for the cause, then `forge service restart`"),
        ),
        "inactive" => check(
            Status::Warn,
            "background daemon",
            "installed but stopped",
            Some("`forge service start`"),
        ),
        other => check(Status::Warn, "background daemon", other.to_string(), None),
    }
}

/// LIVE: for each KEYED provider, can we actually list its models within a timeout? A keyed
/// provider whose discovery times out silently drops out of routing and the mesh falls back to a
/// keyless default (the "groq for everything" churn) — a key-PRESENCE check can't see this.
async fn provider_reachability_checks() -> Vec<Check> {
    const REACH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(8);
    // Probe every keyed provider CONCURRENTLY — each is an independent network call, so a sequential
    // loop made doctor pay the SUM of every provider's timeout (N keyed providers × 8s). join_all
    // collapses that to the slowest single probe; results stay in provider order.
    let probes = forge_config::known_key_providers()
        .filter(|p| forge_config::has_api_key(p))
        .map(|p| async move {
            let res = tokio::time::timeout(REACH_TIMEOUT, forge_provider::list_models(p)).await;
            match res {
            Ok(Ok(list)) if !list.is_empty() => check(
                Status::Ok,
                &format!("{p} reachability"),
                format!("{} models", list.len()),
                None,
            ),
            // Reachable but empty listing — chat may still work; not actionable, so Info.
            Ok(Ok(_)) => check(
                Status::Info,
                &format!("{p} reachability"),
                "responded, but listed no models",
                None,
            ),
            // An Err from list_models is NOT a reliable usability signal: several adapters (Gemini,
            // Groq, the cerebras custom endpoint) have no listing endpoint or key it differently
            // than chat, so they error here while chat works fine. Surface it as Info, not a
            // failure — the robust "this provider is dead" signal is the TIMEOUT branch below, and
            // real credential validation would need a paid chat call doctor won't make by default.
            Ok(Err(_)) => check(
                Status::Info,
                &format!("{p} reachability"),
                "model listing unavailable — chat unaffected",
                None,
            ),
            // The churn cause: keyed but unreachable. Its models won't route.
            Err(_) => check(
                Status::Fail,
                &format!("{p} reachability"),
                format!("discovery timed out (> {}s)", REACH_TIMEOUT.as_secs()),
                Some(
                    "provider/network unreachable — its models won't route this session; the mesh \
                     falls back to another provider",
                ),
            ),
            }
        });
    futures::future::join_all(probes).await
}

/// LIVE: for each AVAILABLE CLI bridge, actually launch it with a tiny prompt and confirm it
/// answers — exercising the real launch path (the Windows `cmd /S /C` shim, auth, the streamed
/// handshake). "On PATH" is not "works": a bridge can resolve on PATH yet fail every turn at
/// launch. $0 on a subscription bridge. Bounded by a timeout so a hung CLI can't wedge doctor.
///
/// A launch failure or timeout here is `Warn`, not `Fail`: CLI bridges are OPTIONAL providers
/// (see `provider_checks()`, which reports a missing bridge as Info "optional"), and an
/// installed-but-unresponsive bridge (e.g. one that needs an interactive login, or is just slow)
/// is a perfectly normal state on an otherwise healthy install — it shouldn't flip doctor's exit
/// code. The hard gate for "nothing usable at all" is the separate `has_usable_provider` check.
async fn bridge_roundtrip_checks() -> Vec<Check> {
    const BRIDGE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
    use forge_provider::Provider as _;
    // Launch every available bridge CONCURRENTLY — each spawns its OWN independent subprocess, so a
    // sequential loop made doctor pay the SUM of every bridge's 30s budget (3 bridges ≈ 90s worst
    // case). join_all collapses that to the slowest single launch; results stay in CliKind order.
    let probes = forge_provider::CliKind::all()
        .into_iter()
        .filter(|k| k.available())
        .map(|k| async move {
            // harness=false → a plain CLI turn (no Forge-tool MCP bridge): the cheapest probe that
            // still exercises the binary launch + auth + a streamed reply.
            let provider = forge_provider::CliProvider::new(k)
                .with_harness(false)
                .with_timeout(BRIDGE_TIMEOUT);
            let model = k.default_model_id();
            let msgs = [forge_types::Message::user("Reply with the single word: ok")];
            let mut sink = |_ev: forge_provider::StreamEvent| {};
            let fut = provider.complete(&model, &msgs, &[], &mut sink);
            let label = format!("{} turn", k.prefix());
            let fix = match k {
                forge_provider::CliKind::ClaudeCode => {
                    "run `claude` once to log in; if it's a Windows .cmd shim, confirm it launches"
                }
                forge_provider::CliKind::Codex => "run `codex login`; confirm the binary launches",
                forge_provider::CliKind::Antigravity => {
                    "check the reported `agy` error; confirm the binary launches"
                }
            };
            match tokio::time::timeout(BRIDGE_TIMEOUT + std::time::Duration::from_secs(2), fut)
                .await
            {
                Ok(Ok(resp)) if !resp.content.trim().is_empty() => {
                    check(Status::Ok, &label, "launches + answers", None)
                }
                Ok(Ok(_)) => check(
                    Status::Warn,
                    &label,
                    "launched but returned no text",
                    Some(fix),
                ),
                Ok(Err(e)) => check(
                    Status::Warn,
                    &label,
                    format!("launch failed: {}", short(&e.to_string())),
                    Some(fix),
                ),
                Err(_) => check(
                    Status::Warn,
                    &label,
                    "timed out — bridge did not respond (needs login?)",
                    Some(fix),
                ),
            }
        });
    futures::future::join_all(probes).await
}

#[cfg(test)]
mod store_check_tests {
    use super::*;

    /// Doctor is what people run WHEN the store is the suspect, so describing a different store
    /// than the session would open is the one answer it must never give. It previously ignored
    /// FORGE_DB and always reported on `data_dir()/forge.db`.
    #[test]
    fn store_path_prefers_forge_db_over_the_default_location() {
        let data = Some(std::path::PathBuf::from("/data"));
        assert_eq!(
            resolve_store_path(Some("/tmp/iso/forge.db"), data.clone()),
            Some(std::path::PathBuf::from("/tmp/iso/forge.db")),
            "an explicit FORGE_DB must win"
        );
        for blank in [None, Some(""), Some("   ")] {
            assert_eq!(
                resolve_store_path(blank, data.clone()),
                Some(std::path::PathBuf::from("/data/forge.db")),
                "a blank FORGE_DB must fall back to the default"
            );
        }
        assert_eq!(resolve_store_path(None, None), None);
    }

    #[test]
    fn a_store_migrated_by_a_newer_build_fails_with_the_remedy() {
        // The exact shape of today's outage: a dev build bumped user_version past what this binary
        // supports, `forge serve` crash-looped, and the Anywhere connector died quietly. Doctor has
        // to name it and say what to do.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("forge.db");
        forge_store::Store::open(&path).unwrap();
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.pragma_update(None, "user_version", 9_999i64).unwrap();
        drop(conn);

        let result = store_check_at(&path);
        assert!(matches!(result.status, Status::Fail));
        assert!(
            result.detail.contains("newer than this build supports"),
            "detail should name the mismatch, got: {}",
            result.detail
        );
        assert!(
            result
                .fix
                .as_deref()
                .is_some_and(|f| f.contains("forge update")),
            "a failure the user cannot act on is not a diagnostic"
        );
    }

    #[test]
    fn a_healthy_store_passes_and_a_missing_one_is_not_a_failure() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("forge.db");
        assert!(matches!(store_check_at(&path).status, Status::Ok));

        forge_store::Store::open(&path).unwrap();
        assert!(matches!(store_check_at(&path).status, Status::Ok));
    }
}

#[cfg(test)]
mod daemon_tests {
    use super::{daemon_state_check, Status};

    /// A restart loop is the case that took the daemon down for 632 restarts while every view
    /// reported it as coming up. It must be a hard failure, not a warning.
    #[test]
    fn a_restart_loop_is_a_hard_failure_with_a_next_step() {
        for state in ["activating", "auto-restart"] {
            let c = daemon_state_check(state);
            assert_eq!(c.status, Status::Fail, "{state} must fail");
            assert!(c.fix.is_some(), "{state} must carry a next step");
        }
    }

    #[test]
    fn running_is_ok_and_stopped_is_only_a_warning() {
        assert_eq!(daemon_state_check("active").status, Status::Ok);
        // Installed-but-stopped is a choice, not a fault: warn with the start command.
        let stopped = daemon_state_check("inactive");
        assert_eq!(stopped.status, Status::Warn);
        assert!(stopped.fix.unwrap().contains("forge service start"));
    }

    #[test]
    fn failed_reports_where_to_look() {
        let c = daemon_state_check("failed");
        assert_eq!(c.status, Status::Fail);
        assert!(c.fix.unwrap().contains("journalctl"));
    }

    /// An unrecognised state word must not be silently treated as healthy.
    #[test]
    fn an_unknown_state_is_surfaced_rather_than_assumed_fine() {
        let c = daemon_state_check("reloading");
        assert_eq!(c.status, Status::Warn);
        assert_eq!(c.detail, "reloading");
    }
}
