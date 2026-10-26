//! Local runtime, setup, and onboarding commands.

use crate::*;
use anyhow::{Context, Result};

mod auth;
pub(crate) use auth::{auth, auth_codex_oauth, auth_xai_oauth, clear_auth_exclusion_in};

/// Render a whole number of seconds as the coarsest useful unit (`"54m"`, `"3h"`, `"2d"`).
fn human_secs(secs: i64) -> String {
    let secs = secs.max(0);
    if secs < 3600 {
        format!("{}m", (secs / 60).max(1))
    } else if secs < 86_400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86_400)
    }
}

/// A human label + free/paid hint for a key-based provider (the table lives with the onboarding
/// copy so the setup screens and `forge auth` never disagree).
pub(crate) fn provider_label(provider: &str) -> &'static str {
    forge_onboarding::providers::provider_label(provider)
}

/// Read one trimmed line from stdin with a prompt (no echo suppression — same as `auth`).
/// Opt-in: if `[local] autostart` is set, ensure the configured local model's Ollama server is up
/// before the chat starts. Best-effort and non-fatal — a failure just means the mesh won't have the
/// local model this session.
pub(crate) fn maybe_autostart_local() {
    let cfg = match super::load_config() {
        Ok(cfg) => cfg,
        Err(error) => {
            eprintln!("⚠ local runtime disabled: {error:#}");
            return;
        }
    };
    if !cfg.local.autostart || !local::ollama_installed() {
        return;
    }
    if local::ollama_start_serve() {
        if let Some(tag) = &cfg.local.model {
            if !local::ollama_installed_models().iter().any(|m| m == tag) {
                println!("⚒ local: pulling {tag} (first run)…");
                local::ollama_pull(tag);
            }
            println!("⚒ local model ready: ollama::{tag}");
        }
    }
}

/// The animated `forge local` menu (no-arg on a terminal): pick a model to install/start, or view
/// status. Loops until the user closes it; each action prints, then waits for Enter before the
/// menu redraws (it owns its own alternate screen).
pub(crate) async fn local_menu() -> Result<()> {
    enum Act {
        Model(String),
        Status,
        Close,
    }
    let scores = local_bench_scores().await;
    loop {
        let specs = local::detect_specs();
        let cands = local::discover_ranked(&specs, scores.as_ref()).await;
        let installed = if local::ollama_installed() {
            local::ollama_installed_models()
        } else {
            Vec::new()
        };
        let mut items: Vec<forge_tui::SelectItem> = Vec::new();
        let mut acts: Vec<Act> = Vec::new();
        for c in &cands {
            let have = installed.iter().any(|t| t == &c.ollama_tag);
            let bench = if c.benchmarked {
                format!("AA {:.0}", c.score)
            } else {
                "—".to_string()
            };
            items.push(forge_tui::SelectItem {
                label: c.label.clone(),
                hint: format!(
                    "{} · ~{:.0} GB · bench {bench}{}",
                    c.ollama_tag,
                    c.min_memory_gb,
                    if have {
                        " · installed → start"
                    } else {
                        " → install"
                    }
                ),
                preselected: false,
            });
            acts.push(Act::Model(c.ollama_tag.clone()));
        }
        items.push(forge_tui::SelectItem {
            label: "Status".into(),
            hint: "runtime + installed models + autostart".into(),
            preselected: false,
        });
        acts.push(Act::Status);
        items.push(forge_tui::SelectItem {
            label: "Close".into(),
            hint: String::new(),
            preselected: false,
        });
        acts.push(Act::Close);

        let title = format!(
            "forge local — {:.0} GB usable · {} · GPU: {} · ranked by Artificial Analysis",
            specs.model_memory_gb(),
            specs.os,
            specs
                .gpu
                .as_ref()
                .map(|g| g.name.as_str())
                .unwrap_or("none")
        );
        let Some(idx) = forge_tui::select_one(&title, &items)? else {
            return Ok(());
        };
        match &acts[idx] {
            Act::Close => return Ok(()),
            Act::Status => {
                local_status();
                let _ = prompt_line("\n  press Enter to continue…");
            }
            Act::Model(tag) => {
                let have = local::ollama_installed_models().iter().any(|t| t == tag);
                let res = if have {
                    local_start(Some(tag))
                } else {
                    local_install(Some(tag))
                };
                if let Err(e) = res {
                    println!("⚠ {e}");
                }
                let _ = prompt_line("\n  press Enter to continue…");
            }
        }
    }
}

/// Artificial Analysis benchmark scores for ranking local models (cache-first; `None` if disabled
/// or unavailable). Seeds the coverage check with the static catalog's tags.
pub(crate) async fn local_bench_scores() -> Option<forge_mesh::BenchmarkScores> {
    let cfg = match super::load_config() {
        Ok(cfg) => cfg,
        Err(error) => {
            eprintln!("⚠ local benchmark ranking disabled: {error:#}");
            return None;
        }
    };
    let ids: Vec<String> = local::CATALOG
        .iter()
        .map(|m| format!("ollama::{}", m.ollama_tag))
        .collect();
    benchmarks::ensure(&cfg, &ids, false).await
}

/// `forge local [subcommand]`: detect specs, install/run a local model via Ollama, list, status.
/// No subcommand on a terminal → the animated interactive menu; otherwise (piped) → `detect`.
pub(crate) async fn local_cmd(sub: Option<LocalCmd>) -> Result<()> {
    let Some(sub) = sub else {
        use std::io::IsTerminal;
        if std::io::stdout().is_terminal() && std::io::stdin().is_terminal() {
            return local_menu().await;
        }
        print_specs_and_recommendation().await;
        return Ok(());
    };
    match sub {
        LocalCmd::Detect => {
            print_specs_and_recommendation().await;
            Ok(())
        }
        LocalCmd::Install { key } => local_install(key.as_deref()),
        LocalCmd::List => {
            if !local::ollama_installed() {
                println!("Ollama is not installed. Run `forge local install` to set it up.");
                return Ok(());
            }
            let models = local::ollama_installed_models();
            if models.is_empty() {
                println!("No local models pulled yet. Run `forge local install`.");
            } else {
                println!("Local models ({}):", models.len());
                for m in models {
                    println!("  • {m}");
                }
            }
            Ok(())
        }
        LocalCmd::Start { key } => local_start(key.as_deref()),
        LocalCmd::Status => {
            local_status();
            Ok(())
        }
    }
}

/// Print the detected specs + the ranked recommendation list.
pub(crate) async fn print_specs_and_recommendation() {
    let specs = local::detect_specs();
    let gpu = match &specs.gpu {
        Some(g) => match g.vram_gb {
            Some(v) => format!("{} ({v:.0} GB VRAM)", g.name),
            None => g.name.clone(),
        },
        None => "none detected".to_string(),
    };
    println!("⚒ This machine");
    println!(
        "  RAM {:.0} GB · {} cores · {} · GPU: {gpu}",
        specs.total_ram_gb, specs.cpu_cores, specs.os
    );
    println!(
        "  model memory budget: ~{:.0} GB\n",
        specs.model_memory_gb()
    );

    let scores = local_bench_scores().await;
    let cands = local::discover_ranked(&specs, scores.as_ref()).await;
    if cands.is_empty() {
        println!("No model fits this machine's memory (the smallest needs ~4 GB).");
        return;
    }
    let benched = cands.iter().filter(|c| c.benchmarked).count();
    println!(
        "Models that fit, ranked by Artificial Analysis benchmark score ({benched}/{} rated):",
        cands.len()
    );
    for (i, c) in cands.iter().enumerate() {
        let rec = if i == 0 { "  ‹recommended›" } else { "" };
        let bench = if c.benchmarked {
            format!("AA {:.0}", c.score)
        } else {
            "unrated".to_string()
        };
        println!(
            "  {} {:<26} [{}]  {} · ~{:.0} GB · {bench}{rec}",
            if i == 0 { "▸" } else { " " },
            c.label,
            c.ollama_tag,
            c.family,
            c.min_memory_gb,
        );
        if !c.blurb.is_empty() {
            println!("      {}", c.blurb);
        }
    }
    println!(
        "\nInstall with `forge local install` (recommended) or `forge local install <tag-or-key>`."
    );
}

/// Ensure Ollama is installed (offering to install it), then pull the chosen (or recommended)
/// model. `name` is a raw Ollama tag (`qwen2.5-coder:14b`), a catalog key (`qwen2.5-coder-14b`),
/// or `None` for the recommended pick.
pub(crate) fn local_install(name: Option<&str>) -> Result<()> {
    let specs = local::detect_specs();
    // Resolve to (display label, ollama tag).
    let (label, tag): (String, String) = match name {
        Some(n) if n.contains(':') => (n.to_string(), n.to_string()), // raw tag
        Some(k) => {
            let m = local::model_by_key(k)
                .with_context(|| format!("unknown model '{k}' — see `forge local detect`"))?;
            (m.label.to_string(), m.ollama_tag.to_string())
        }
        None => {
            let m = *local::recommend(&specs)
                .first()
                .context("no local model fits this machine (needs ≥4 GB)")?;
            (m.label.to_string(), m.ollama_tag.to_string())
        }
    };

    if !local::ollama_installed() {
        println!("Ollama (the local-model runtime) is not installed.");
        match local::ollama_install_command(&specs) {
            Some((cmd, args)) => {
                let shown = std::iter::once(cmd.to_string())
                    .chain(args.iter().cloned())
                    .collect::<Vec<_>>()
                    .join(" ");
                let yes = prompt_line(&format!("Install it now with `{shown}`? [Y/n]: "))?;
                if yes.is_empty()
                    || yes.eq_ignore_ascii_case("y")
                    || yes.eq_ignore_ascii_case("yes")
                {
                    if !local::run_install(cmd, &args) {
                        anyhow::bail!("Ollama install failed — install it manually from https://ollama.com/download, then re-run.");
                    }
                } else {
                    println!("Skipped. Install Ollama from https://ollama.com/download, then re-run `forge local install`.");
                    return Ok(());
                }
            }
            None => {
                println!("Install Ollama from https://ollama.com/download, then re-run `forge local install`.");
                return Ok(());
            }
        }
    }

    println!("Pulling {label} ({tag})…");
    if !local::ollama_pull(&tag) {
        anyhow::bail!(
            "`ollama pull {tag}` failed. The tag may not exist in your Ollama version — check `ollama list` / upgrade Ollama, or pick another model with `forge local detect`."
        );
    }
    println!("✓ {label} is ready. It's available in the mesh as `ollama::{tag}`.");
    println!("  Start it with `forge local start {tag}`, or enable `[local] autostart` in config.");
    Ok(())
}

/// Ensure the Ollama server is up and the chosen model is available.
pub(crate) fn local_start(key: Option<&str>) -> Result<()> {
    if !local::ollama_installed() {
        anyhow::bail!("Ollama is not installed. Run `forge local install` first.");
    }
    let cfg = super::load_config()?;
    // Choose the model: raw tag as-is; catalog key → its tag; else configured tag; else recommended.
    let tag: String = match key {
        Some(n) if n.contains(':') => n.to_string(),
        Some(k) => local::model_by_key(k)
            .map(|m| m.ollama_tag.to_string())
            .with_context(|| format!("unknown model '{k}'"))?,
        None => cfg
            .local
            .model
            .clone()
            .or_else(|| {
                let specs = local::detect_specs();
                local::recommend(&specs)
                    .first()
                    .map(|m| m.ollama_tag.to_string())
            })
            .context("no model configured and none fits — run `forge local install`")?,
    };
    print!("Starting Ollama… ");
    std::io::Write::flush(&mut std::io::stdout()).ok();
    if !local::ollama_start_serve() {
        anyhow::bail!("could not start `ollama serve` (is it already running on another port?)");
    }
    println!("up.");
    if !local::ollama_installed_models().iter().any(|m| m == &tag) {
        println!("Model {tag} not pulled yet — pulling…");
        if !local::ollama_pull(&tag) {
            anyhow::bail!("`ollama pull {tag}` failed.");
        }
    }
    println!("✓ Local model ready: `ollama::{tag}` (mesh will route to it).");
    Ok(())
}

/// Print local-runtime status: install, serving, models, and the autostart config.
pub(crate) fn local_status() {
    let cfg = match super::load_config() {
        Ok(cfg) => cfg,
        Err(error) => {
            eprintln!("⚠ cannot load local runtime configuration: {error:#}");
            return;
        }
    };
    match local::ollama_version() {
        Some(v) => println!("Ollama: installed ({v})"),
        None => {
            println!("Ollama: not installed — run `forge local install`");
            return;
        }
    }
    println!(
        "Server:  {}",
        if local::ollama_serving() {
            "running (localhost:11434)"
        } else {
            "stopped — `forge local start`"
        }
    );
    let models = local::ollama_installed_models();
    println!(
        "Models:  {}",
        if models.is_empty() {
            "none".to_string()
        } else {
            models.join(", ")
        }
    );
    println!(
        "Autostart: {}{}",
        if cfg.local.autostart { "on" } else { "off" },
        cfg.local
            .model
            .as_deref()
            .map(|m| format!(" · model {m}"))
            .unwrap_or_default()
    );
}

pub(crate) fn prompt_line(prompt: &str) -> Result<String> {
    print!("{prompt}");
    std::io::Write::flush(&mut std::io::stdout()).ok();
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .context("reading stdin")?;
    Ok(line.trim().to_string())
}

/// Refusal message for the guided setup on a non-tty. Names the command the user actually typed
/// (`forge setup` and `forge init` are aliases) and the unattended alternative.
pub(crate) fn non_interactive_message(invoked_as: &str) -> String {
    format!(
        "`{invoked_as}` is interactive — run it in a terminal, or run `forge auth <provider>` \
         for unattended setup"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_non_interactive_refusal_names_the_alias_the_user_typed() {
        let setup = non_interactive_message("forge setup");
        assert!(setup.starts_with("`forge setup` is interactive"), "{setup}");
        assert!(!setup.contains("forge init"), "{setup}");
        assert!(setup.contains("forge auth <provider>"), "{setup}");
        assert!(non_interactive_message("forge init").starts_with("`forge init` is interactive"));
    }

    /// The Codex authorize request must match OpenAI's registered public client
    /// (`app_EMoamEEZ73f0CkXaXp7hrann`) byte-exact, or Hydra rejects it with
    /// `authorize_hydra_invalid_request` before consent even renders: the `localhost` (not
    /// `127.0.0.1`) redirect_uri, plus the two Codex-CLI-specific params.
    #[test]
    fn codex_authorize_url_matches_registered_client() {
        let url = auth::codex_authorize_url("test-state", "test-challenge");
        assert!(
            url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback"),
            "expected percent-encoded localhost redirect_uri in {url}"
        );
        assert!(
            url.contains("id_token_add_organizations=true"),
            "missing id_token_add_organizations param in {url}"
        );
        assert!(
            url.contains("codex_cli_simplified_flow=true"),
            "missing codex_cli_simplified_flow param in {url}"
        );
    }

    /// The store side of re-authentication: a corroborated auth failure excludes the whole
    /// provider, and a user who has just fixed the credential expects the next turn to work rather
    /// than to wait for the window or a probe. A rate-limit bench on a sibling must survive — a new
    /// key does not reset someone else's quota.
    #[test]
    fn reauthenticating_clears_the_provider_auth_exclusion_but_keeps_unrelated_benches() {
        let store = Store::open_in_memory().unwrap();
        store.exclude_provider("openai", "auth failed").unwrap();
        store
            .exclude_model("openai::gpt-5-mini", "auth failed")
            .unwrap();
        store
            .bench_for(
                "openai::gpt-5",
                std::time::Duration::from_secs(600),
                "rate-limited",
            )
            .unwrap();
        store
            .bench_for(
                "groq::llama-4",
                std::time::Duration::from_secs(600),
                "auth failed",
            )
            .unwrap();

        assert!(
            store
                .current_benched()
                .unwrap()
                .is_benched("openai::gpt-5.5"),
            "the provider exclusion must bench every openai alias before the fix"
        );

        clear_auth_exclusion_in(&store, "openai");

        let health = store.current_benched().unwrap();
        assert!(
            !health.is_benched("openai::gpt-5.5"),
            "a re-authenticated provider must be routable again"
        );
        assert!(
            !health.is_benched("openai::gpt-5-mini"),
            "the per-model auth exclusion must go too"
        );
        assert!(
            health.is_benched("openai::gpt-5"),
            "a rate-limit bench is not a credential problem and must survive re-auth"
        );
        assert!(
            health.is_benched("groq::llama-4"),
            "another provider's exclusion must be untouched"
        );
    }
}
