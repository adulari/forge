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

use crate::local;

/// One diagnostic line's outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
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

struct Check {
    status: Status,
    label: String,
    detail: String,
    /// An actionable next step, shown when not `Ok`.
    fix: Option<String>,
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

fn check(status: Status, label: &str, detail: impl Into<String>, fix: Option<&str>) -> Check {
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
    // Live, timeout-bounded: prove keyed providers are actually reachable (not just key-present).
    provider_v.extend(provider_reachability_checks().await);
    sections.push(("Providers", provider_v));
    // Live, timeout-bounded: prove a detected bridge can actually launch + answer (not just on PATH).
    let bridge_live = bridge_roundtrip_checks().await;
    if !bridge_live.is_empty() {
        sections.push(("Bridge liveness", bridge_live));
    }
    sections.push(("Background daemon", daemon_checks()));
    sections.push(("Local LLM (Ollama)", ollama_checks()));
    sections.push(("Environment", environment_checks()));

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

fn environment_checks() -> Vec<Check> {
    use std::io::IsTerminal;
    let mut out = Vec::new();

    // git
    let git = binary_on_path("git");
    out.push(check(
        if git { Status::Ok } else { Status::Warn },
        "git",
        if git { "on PATH" } else { "not found" },
        (!git).then_some("install git — some features (provenance, /init) use it"),
    ));
    if git {
        let in_repo = std::process::Command::new("git")
            .args(["rev-parse", "--is-inside-work-tree"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        out.push(check(
            Status::Info,
            "git repo",
            if in_repo {
                "inside a work tree"
            } else {
                "not in a git repo (cwd)"
            },
            None,
        ));
    }

    // terminal — resolve the old "(?)". On Unix an interactive stdout with no usable TERM is the
    // class of box where the full-screen TUI misbehaves, so flag it. On Windows TERM is a Unix
    // concept and is normally UNSET — crossterm drives the console via the Console API regardless —
    // so an interactive Windows console is simply OK (warning there is a false positive).
    let tty = std::io::stdout().is_terminal();
    let term = std::env::var("TERM").ok().filter(|t| !t.is_empty());
    let term_usable = term.as_deref().is_some_and(|t| t != "dumb");
    // Gold-standard viability: actually enter+exit raw mode, exactly what the TUI does on launch.
    // More authoritative than the TERM heuristic — a box where this fails genuinely can't run the
    // full-screen UI, while one where it succeeds can (even with an odd TERM). Only meaningful on an
    // interactive stdout, so it's gated on `tty`.
    let raw_probe = tty.then(raw_mode_probe);
    let (status, detail, fix) = if !tty {
        (Status::Info, "non-interactive (piped/CI)".to_string(), None)
    } else if let Some(Err(e)) = &raw_probe {
        (
            Status::Warn,
            format!("interactive but raw-mode probe failed ({e}) — the full-screen TUI won't work here"),
            Some("use a different terminal emulator; ensure stdin+stdout are a real tty and TERM is set"),
        )
    } else if cfg!(windows) {
        (
            Status::Ok,
            "interactive (Windows console, raw-mode OK)".to_string(),
            None,
        )
    } else if let Some(term) = term.as_deref().filter(|_| term_usable) {
        (
            Status::Ok,
            format!("interactive ({term}, raw-mode OK)"),
            None,
        )
    } else {
        (
            Status::Warn,
            format!(
                "interactive but TERM={} — the TUI may not render correctly",
                term.as_deref().unwrap_or("(unset)")
            ),
            Some("export TERM=xterm-256color (add it to your shell profile)"),
        )
    };
    out.push(check(status, "terminal", detail, fix));

    // WSL: surface it explicitly — it's the platform behind most "hangs / won't open" reports, and
    // knowing it's WSL focuses the fix (TERM, a responsive keyring, PATH'd Windows .cmd shims).
    if is_wsl() {
        out.push(check(Status::Info, "platform", "WSL detected", None));
    }
    out
}

/// Enter then exit raw mode — the exact terminal capability the full-screen TUI needs. Returns the
/// error string if entering fails (a box that can't support the UI). Always attempts to restore
/// cooked mode so `forge doctor` never leaves the terminal in raw mode.
fn raw_mode_probe() -> Result<(), String> {
    use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
    enable_raw_mode().map_err(|e| e.to_string())?;
    disable_raw_mode().map_err(|e| e.to_string())
}

/// Best-effort WSL detection: the kernel release string carries "microsoft" under WSL1/2.
fn is_wsl() -> bool {
    std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .map(|s| s.to_lowercase().contains("microsoft"))
        .unwrap_or(false)
}

/// Truncate a provider/error string to one tidy line for the report.
fn short(s: &str) -> String {
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
fn daemon_checks() -> Vec<Check> {
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

    #[cfg(target_os = "linux")]
    if let Some(unit) = crate::cli::commands::service::installed_unit_text() {
        let exe = std::env::current_exe()
            .ok()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        out.extend(unit_drift_checks(&unit, &exe));
    }
    out
}

/// Does the installed unit match what THIS binary would render?
///
/// `forge service install` writes the unit once; upgrading Forge never rewrites it. So a fix that
/// lives in the unit rather than the binary silently never reaches an existing install. That is not
/// hypothetical: #996 stops the daemon restart-looping on an unfixable failure via BOTH an exit code
/// (binary) and `RestartPreventExitStatus=78` (unit). Upgrade alone delivers half of it, and the
/// daemon keeps looping — 25,218 consecutive restarts on the author's machine, silently.
///
/// Pure so every branch is testable without installing or breaking a real service.
fn unit_drift_checks(unit: &str, current_exe: &str) -> Vec<Check> {
    let mut out = Vec::new();

    if !unit.contains("RestartPreventExitStatus") {
        out.push(check(
            Status::Warn,
            "daemon unit",
            "predates the permanent-failure guard — a failure retrying cannot fix will restart forever",
            Some("`forge service install` to re-render the unit (it is rewritten only on install)"),
        ));
    }

    // ExecStart pins an absolute path. Install Forge somewhere else afterwards (brew, cargo) and
    // systemd keeps launching the OLD binary, so `forge --version` in a terminal can disagree with
    // what the daemon is actually running, with nothing anywhere saying so.
    let unit_exe = unit
        .lines()
        .find_map(|l| l.trim().strip_prefix("ExecStart="))
        .and_then(|cmd| cmd.split_whitespace().next())
        .unwrap_or_default();
    // A dev build is EXPECTED to differ from the installed unit — warning about it every time
    // someone runs `cargo run -- doctor` would be pure noise, and noise is how the real signal
    // above gets ignored.
    let from_build_tree =
        current_exe.contains("/target/debug/") || current_exe.contains("/target/release/");
    if !unit_exe.is_empty()
        && !current_exe.is_empty()
        && unit_exe != current_exe
        && !from_build_tree
    {
        out.push(check(
            Status::Warn,
            "daemon binary",
            format!("unit runs {unit_exe}, but this is {current_exe}"),
            Some("`forge service install` to point the unit at this binary"),
        ));
    }

    if out.is_empty() {
        out.push(check(
            Status::Ok,
            "daemon unit",
            "matches this binary",
            None,
        ));
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
                &format!("{p} reachable"),
                format!("{} models", list.len()),
                None,
            ),
            // Reachable but empty listing — chat may still work; not actionable, so Info.
            Ok(Ok(_)) => check(
                Status::Info,
                &format!("{p} reachable"),
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
                &format!("{p} reachable"),
                "model listing unavailable — chat unaffected",
                None,
            ),
            // The churn cause: keyed but unreachable. Its models won't route.
            Err(_) => check(
                Status::Fail,
                &format!("{p} reachable"),
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
                    "run `agy` once to log in; confirm the binary launches"
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

fn binary_on_path(bin: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|d| d.join(bin).is_file()))
        .unwrap_or(false)
}

#[cfg(test)]
mod unit_drift_tests {
    use super::{unit_drift_checks, Status};

    /// The unit actually installed on the author's machine on 2026-08-10, verbatim. Its daemon had
    /// restarted 25,218 times; #996's exit-code half would have arrived with an upgrade and changed
    /// nothing, because this unit has no `RestartPreventExitStatus`.
    const REAL_STALE_UNIT: &str = "[Unit]\nDescription=Forge serve\n\n[Service]\n\
        ExecStart=/home/floris/.local/bin/forge serve --tunnel --port 7420\nRestart=on-failure\n";

    #[test]
    fn a_unit_without_the_permanent_failure_guard_is_flagged() {
        let out = unit_drift_checks(REAL_STALE_UNIT, "/home/floris/.local/bin/forge");
        let guard = out
            .iter()
            .find(|c| c.label == "daemon unit")
            .expect("the missing guard must be reported");
        assert_eq!(guard.status, Status::Warn);
        assert!(guard.fix.is_some(), "must say how to fix it");
        // Same binary path, so only the guard should be reported — not a spurious binary mismatch.
        assert!(out.iter().all(|c| c.label != "daemon binary"));
    }

    #[test]
    fn a_unit_pointing_at_another_binary_is_flagged() {
        let unit = "[Service]\nExecStart=/usr/bin/forge serve --port 7420\n\
            RestartPreventExitStatus=78\nRestart=on-failure\n";
        let out = unit_drift_checks(unit, "/home/floris/.cargo/bin/forge");
        let drift = out
            .iter()
            .find(|c| c.label == "daemon binary")
            .expect("a different ExecStart path must be reported");
        assert_eq!(drift.status, Status::Warn);
        assert!(drift.detail.contains("/usr/bin/forge"), "{}", drift.detail);
        assert!(drift.detail.contains("cargo"), "{}", drift.detail);
    }

    #[test]
    fn a_current_unit_reports_ok_and_nothing_else() {
        let unit = "[Service]\nExecStart=/usr/bin/forge serve --port 7420\n\
            RestartPreventExitStatus=78\nRestart=on-failure\n";
        let out = unit_drift_checks(unit, "/usr/bin/forge");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].status, Status::Ok);
    }

    /// An unknown current-exe path must not manufacture a mismatch — reporting drift against an
    /// empty string would fire on every install that cannot resolve its own binary.
    #[test]
    fn an_unresolvable_current_binary_does_not_report_drift() {
        let unit = "[Service]\nExecStart=/usr/bin/forge serve\nRestartPreventExitStatus=78\n";
        let out = unit_drift_checks(unit, "");
        assert!(out.iter().all(|c| c.label != "daemon binary"));
    }

    /// A dev build always differs from the installed unit. Warning every `cargo run -- doctor` is
    /// noise, and noise is how the genuine warning above gets ignored.
    #[test]
    fn a_build_tree_binary_does_not_report_drift() {
        let unit = "[Service]\nExecStart=/usr/bin/forge serve\nRestartPreventExitStatus=78\n";
        for exe in [
            "/home/me/src/forge/target/debug/forge",
            "/home/me/src/forge/target/release/forge",
        ] {
            let out = unit_drift_checks(unit, exe);
            assert!(
                out.iter().all(|c| c.label != "daemon binary"),
                "{exe} must not warn"
            );
        }
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
