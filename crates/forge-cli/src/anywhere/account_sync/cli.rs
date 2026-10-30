//! `forge anywhere sync`: status, per-category switches and an immediate cycle.

use std::io::{IsTerminal, Write as _};

use anyhow::{bail, Context, Result};

use super::{status_snapshot, AccountSyncStatus, Category};
use crate::SyncCmd;

pub(crate) async fn sync_cmd(command: Option<SyncCmd>) -> Result<()> {
    match command.unwrap_or(SyncCmd::Status) {
        SyncCmd::Status => {
            print_status(&status_snapshot(&crate::open_store()?)?);
            Ok(())
        }
        SyncCmd::Enable { category, yes } => set(&category, true, yes),
        SyncCmd::Disable { category } => set(&category, false, true),
        SyncCmd::Now => now().await,
    }
}

fn set(category: &str, enabled: bool, yes: bool) -> Result<()> {
    let Some(parsed) = Category::parse(category) else {
        bail!("unknown category `{category}`; choose one of keys, skills, settings, mcp, instructions");
    };
    if enabled && !crate::anywhere::signed_in() {
        bail!("{}", super::not_signed_in_message());
    }
    if enabled && parsed == Category::Keys && !yes && !confirm_keys()? {
        println!("Left off.");
        return Ok(());
    }
    forge_config::account_sync::set_account_sync_flag(parsed.id(), enabled)?;
    println!(
        "{} sync is now {} on this device. A running `forge serve` applies it within seconds.",
        parsed.label(),
        if enabled { "on" } else { "off" }
    );
    if enabled && parsed == Category::Keys {
        println!("Rotating logins (Codex, xAI, MCP OAuth) never sync; other devices get a sign-in prompt instead.");
    }
    Ok(())
}

fn confirm_keys() -> Result<bool> {
    if !std::io::stdin().is_terminal() {
        bail!("syncing keys needs a person at this terminal; re-run with --yes to confirm");
    }
    println!(
        "Keys & tokens sync uploads your provider API keys, end-to-end encrypted, so every device"
    );
    println!(
        "that opts in receives them. A revoked device keeps ciphertext it already downloaded, so"
    );
    println!("you would rotate keys at the provider after revoking one.");
    print!("Turn on key sync for this device? [y/N] ");
    std::io::stdout().flush().ok();
    let mut answer = String::new();
    std::io::stdin()
        .read_line(&mut answer)
        .context("read confirmation")?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes"))
}

async fn now() -> Result<()> {
    let config = forge_config::load()?;
    let port = config.remote.serve_port();
    let token = crate::serve::daemon_token(false)?;
    let response = reqwest::Client::new()
        .post(format!(
            "http://127.0.0.1:{port}/{token}/api/anywhere/sync/now"
        ))
        .send()
        .await
        .context(
            "reach the local `forge serve` daemon (start it, or wait for the next 10s cycle)",
        )?;
    if !response.status().is_success() {
        bail!(
            "the daemon refused the request (HTTP {})",
            response.status()
        );
    }
    println!("Sync cycle requested.");
    Ok(())
}

pub(crate) fn print_status(status: &AccountSyncStatus) {
    if !status.signed_in {
        println!("Account sync: not signed in to Forge Anywhere");
        return;
    }
    for category in &status.categories {
        let state = if category.enabled { "on " } else { "off" };
        let waiting = match category.remote_only {
            Some(n) if n > 0 && !category.enabled => format!(" · {n} item(s) on other devices"),
            Some(n) if n > 0 => format!(" · {n} waiting to apply"),
            _ => String::new(),
        };
        println!(
            "  {state} {:<16} {} item(s) · {} pending{waiting}",
            category.label, category.items, category.pending
        );
        for conflict in &category.conflicts {
            println!("      conflict: {} ({})", conflict.item, conflict.detail);
        }
        for skipped in &category.skipped {
            println!("      skipped: {} ({})", skipped.item, skipped.reason);
        }
    }
    for sign_in in &status.sign_ins {
        println!("  sign in to {}: {}", sign_in.label, sign_in.command);
    }
    for tool in &status.missing_tools {
        println!("  missing {}: {}", tool.label, tool.hint);
    }
    if let Some(error) = &status.last_error {
        println!("  last error: {error}");
    }
}

/// One-line summary for `forge anywhere status` and the TUI `/anywhere` card.
pub(crate) fn summary_line() -> String {
    let Ok(config) = forge_config::load() else {
        return "Account sync: unavailable".into();
    };
    let on: Vec<&str> = Category::ALL
        .into_iter()
        .filter(|c| c.enabled(&config.anywhere.account_sync))
        .map(Category::id)
        .collect();
    if on.is_empty() {
        "Account sync: off (forge anywhere sync enable <category>)".into()
    } else {
        format!("Account sync: {}", on.join(", "))
    }
}
