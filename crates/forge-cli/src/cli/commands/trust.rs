//! `forge trust` — record that a project's `.forge/` settings may run (docs/features/project-trust.md).

use std::io::{BufRead, IsTerminal, Write};
use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::Command;

pub(crate) fn trust_cmd(path: Option<PathBuf>, revoke: bool, list: bool) -> Result<()> {
    if list {
        let trusted = forge_config::project_trust::trusted_projects();
        if trusted.is_empty() {
            println!("no trusted projects");
        }
        for root in trusted {
            println!("{}", root.display());
        }
        return Ok(());
    }
    let dir = match path {
        Some(path) => path,
        None => std::env::current_dir().context("reading the current directory")?,
    };
    if revoke {
        if forge_config::project_trust::revoke(&dir).context("updating the trust store")? {
            println!(
                "✓ {} is no longer trusted",
                forge_config::project_trust::trust_root(&dir).display()
            );
        } else {
            println!(
                "{} was not trusted",
                forge_config::project_trust::trust_root(&dir).display()
            );
        }
        return Ok(());
    }
    let root = forge_config::project_trust::trust(&dir).context("updating the trust store")?;
    println!(
        "✓ trusted {} — its .forge/ hooks, MCP servers and permission settings now apply",
        root.display()
    );
    Ok(())
}

/// Whether `command` starts an agent session in this directory, so an untrusted project is worth
/// mentioning (a `forge models` listing is not).
fn starts_session(command: &Command) -> bool {
    matches!(
        command,
        Command::Run { .. } | Command::Chat { .. } | Command::McpServe { .. }
    )
}

/// At startup: when the current project ships settings that need trust, ask once on an
/// interactive `forge chat` / `forge run --tui`, and otherwise say what was ignored.
pub(crate) fn startup_trust_gate(command: &Command) {
    if !starts_session(command) {
        return;
    }
    let Some(notice) = forge_config::project_trust::untrusted_notice() else {
        return;
    };
    let interactive_tui = matches!(
        command,
        Command::Chat { .. } | Command::Run { tui: true, .. }
    );
    if interactive_tui && std::io::stdin().is_terminal() && std::io::stderr().is_terminal() {
        eprintln!("⚠ {notice}.");
        eprint!("Trust this folder and load them? [y/N] ");
        let _ = std::io::stderr().flush();
        let mut answer = String::new();
        let _ = std::io::stdin().lock().read_line(&mut answer);
        if matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
            match std::env::current_dir()
                .map_err(anyhow::Error::from)
                .and_then(|dir| forge_config::project_trust::trust(&dir).map_err(Into::into))
            {
                Ok(root) => eprintln!("✓ trusted {}", root.display()),
                Err(error) => eprintln!("could not record trust: {error}"),
            }
        }
        return;
    }
    eprintln!("note: {notice}");
}
