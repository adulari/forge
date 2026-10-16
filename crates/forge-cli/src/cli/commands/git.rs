use anyhow::{Context, Result};

use crate::*;

use forge_core::git_attribution::{self, HookInstall};

fn report_install(outcome: HookInstall) -> Result<()> {
    match outcome {
        HookInstall::Installed(path) => {
            println!("✓ installed {}", path.display());
        }
        HookInstall::Chained { hook, kept } => {
            println!("✓ installed {}", hook.display());
            println!(
                "  your existing hook was kept as {} and runs first",
                kept.display()
            );
        }
        HookInstall::Skipped(why) => anyhow::bail!("{why}"),
        HookInstall::NotARepo => anyhow::bail!("not inside a git repository"),
    }
    println!("  strips Claude/Codex co-author lines; adds Co-Authored-By: Forge (model)");
    Ok(())
}

/// Auto-install the commit hook when `[git] coauthor = true` and we're inside a git repo. Best-
/// effort and idempotent, so a fresh clone gets attribution without the user remembering to run
/// `forge git setup`. Silent on any failure — it's a convenience. (Sessions also install it
/// themselves before their first model request; this covers a commit made before any turn.)
pub(crate) fn maybe_install_git_hook(config: &forge_config::Config) {
    if !config.git.coauthor {
        return;
    }
    if let Ok(cwd) = std::env::current_dir() {
        let _ = git_attribution::install_commit_hook(&cwd);
    }
}

pub(crate) fn git_cmd(cmd: GitCmd) -> Result<()> {
    match cmd {
        GitCmd::Setup { force } => {
            let config = forge_config::load().context("loading forge config")?;
            if !force && !config.git.coauthor {
                anyhow::bail!(
                    "git.coauthor is not enabled in .forge/config.toml\n\
                     Add `[git]\ncoauthor = true` to enable, or run `forge git setup --force`."
                );
            }
            let cwd = std::env::current_dir().context("reading the working directory")?;
            let outcome = git_attribution::install_commit_hook(&cwd).map_err(anyhow::Error::msg)?;
            report_install(outcome)
        }
        GitCmd::Hook => {
            print!("{}", git_attribution::COMMIT_HOOK);
            Ok(())
        }
    }
}
