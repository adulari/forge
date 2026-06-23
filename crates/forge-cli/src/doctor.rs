//! `forge doctor` — diagnose a user's environment in one command: config, providers/keys, CLI
//! bridges, the local Ollama runtime, git, and the terminal — each with an actionable fix. The
//! single biggest lever for onboarding + support (and the first thing to paste into a bug report).
//! All checks are local + fast (no model calls); a couple do a cheap PATH/TCP probe.

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
pub fn run() -> anyhow::Result<usize> {
    println!("⚒ forge doctor — {}\n", env!("CARGO_PKG_VERSION"));

    let mut sections: Vec<(&str, Vec<Check>)> = Vec::new();
    sections.push(("Config", config_checks()));
    let (provider_checks, has_usable_provider) = provider_checks();
    sections.push(("Providers", provider_checks));
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

    // terminal
    let tty = std::io::stdout().is_terminal();
    out.push(check(
        if tty { Status::Ok } else { Status::Info },
        "terminal",
        if tty {
            format!(
                "interactive ({})",
                std::env::var("TERM").unwrap_or_else(|_| "?".into())
            )
        } else {
            "non-interactive (piped/CI)".to_string()
        },
        None,
    ));
    out
}

fn binary_on_path(bin: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|d| d.join(bin).is_file()))
        .unwrap_or(false)
}
