//! Startup and background update checks: a throttled, best-effort release lookup. Privacy-respecting
//! (sends no data beyond the GET), off-switchable (`[update] check = false` or
//! `FORGE_NO_UPDATE_CHECK=1`), and never blocks or fails a session — any error is silently ignored.
//!
//! The plain chat path asks on stdin; the full-screen TUI gets an [`UpdateEvent`] stream it turns
//! into the update card. What was found and what the user decided lives in [`crate::update_state`].

use std::sync::mpsc::Sender;
use std::time::Duration;

use forge_tui::update_card::{ServiceNote, UpdateOffer, UpdatePlan};

use crate::update::InstallMethod;
use crate::update_feed::{fetch_latest, notes_preview, Release};
use crate::update_state::{check_interval, disabled, now, UpdateState};

const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// What the background work tells the TUI shell.
#[derive(Debug, Clone)]
pub enum UpdateEvent {
    Offer(UpdateOffer),
    Progress(String),
    Finished(Result<String, String>),
    Service(ServiceNote),
    /// A line for the transcript (an automatic update happened or finished).
    Notice(String),
}

/// The card's view of `release` for an install managed by `method`.
pub fn offer_for(release: &Release, current: &str, method: InstallMethod) -> UpdateOffer {
    UpdateOffer {
        version: release.version.clone(),
        current: current.to_string(),
        notes: notes_preview(&release.notes, 3, 100),
        url: release.url.clone(),
        plan: match method.upgrade_command() {
            Some(cmd) => UpdatePlan::Command(cmd),
            None => UpdatePlan::InPlace,
        },
    }
}

/// Ask the network if the interval has elapsed (or `force`), fold the answer into the saved
/// state, and return the state. The attempt is recorded first so a hang or an offline run does not
/// retry on every launch; the result is merged into a fresh load so a skip made meanwhile is kept.
pub(crate) async fn refresh_state(force: bool) -> UpdateState {
    let mut state = UpdateState::load();
    if !force && !state.due(now(), check_interval()) {
        return state;
    }
    state.record_attempt(now());
    state.save();
    let found = fetch_latest().await;
    let mut state = UpdateState::load();
    state.record_release(found);
    state.save();
    state
}

/// Print an update notice if a newer release exists. Throttled + best-effort; returns immediately
/// when disabled, recently checked, or offline.
pub async fn maybe_notify(config: &forge_config::Config) {
    if disabled(config) {
        return;
    }
    let state = refresh_state(false).await;
    let Some(release) = state.offer(CURRENT).cloned() else {
        return;
    };
    let latest = &release.version;
    // On an interactive terminal, offer to update right now; otherwise just print the notice. The
    // TTY gate keeps headless runs, pipes, and the `mcp-serve` bridge from ever blocking on input.
    use std::io::{IsTerminal, Write};
    if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
        print!(
            "⚒ Forge {latest} is available (you have {CURRENT}). Update now? [y/N/s=skip this version] "
        );
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).is_ok() {
            match line.trim().to_ascii_lowercase().as_str() {
                "y" | "yes" => {
                    // self_update is blocking — run it off the async runtime so we don't stall
                    // the reactor.
                    match tokio::task::spawn_blocking(|| crate::update::run(false)).await {
                        Ok(Ok(())) => {}
                        Ok(Err(e)) => eprintln!("update failed: {e:#}"),
                        Err(e) => eprintln!("update task panicked: {e}"),
                    }
                    return;
                }
                "s" | "skip" => {
                    let mut state = UpdateState::load();
                    state.skip(latest);
                    state.save();
                    println!("skipping Forge {latest}; the next release will be offered.");
                    return;
                }
                _ => {}
            }
        }
    }
    println!(
        "⚒ Forge {latest} is available (you have {CURRENT}).\n  Update: run `forge update`, or see\n  {}",
        release.url
    );
}

/// TUI-mode update checks: a background task that offers a cached release at once (so a
/// "remind me later" is honoured at the next launch without waiting on the network), then asks the
/// network on startup and every interval while the TUI runs. It never blocks startup, never prints
/// (which would corrupt the TUI), and stops when the TUI's channel is gone.
///
/// With automatic updates on, a newer release is installed quietly instead of offered: a
/// standalone build is staged for the next launch's swap, a Homebrew/Scoop install is upgraded in
/// the background. Only when that does not apply (or fails) does the card appear.
pub fn spawn_checker(config: &forge_config::Config, tx: Sender<UpdateEvent>) {
    if let Some(notice) = crate::update_auto::take_updated_notice() {
        let _ = tx.send(UpdateEvent::Notice(notice));
    }
    if disabled(config) {
        return;
    }
    let auto = crate::update_auto::auto_enabled(config) && crate::update_auto::at_a_terminal();
    tokio::spawn(async move {
        let method = crate::update::detect_install_method();
        let mut announced: Option<String> = None;
        loop {
            let state = refresh_state(false).await;
            if let Some(release) = state.offer(CURRENT) {
                if announced.as_deref() != Some(release.version.as_str()) {
                    announced = Some(release.version.clone());
                    let installed = if auto {
                        install_quietly(method, &release.version).await
                    } else {
                        None
                    };
                    let event = match installed {
                        Some(notice) => UpdateEvent::Notice(notice),
                        None => UpdateEvent::Offer(offer_for(release, CURRENT, method)),
                    };
                    if tx.send(event).is_err() {
                        return;
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(check_interval().max(30))).await;
        }
    });
}

/// Install `version` without asking, when this install allows it. The notice says it takes over
/// next launch; `None` means nothing was installed and the card should offer it instead.
async fn install_quietly(method: InstallMethod, version: &str) -> Option<String> {
    use crate::update_auto::{plan_launch, LaunchAction, LaunchFacts};
    let action = plan_launch(&LaunchFacts {
        interactive: true,
        auto: true,
        method,
        cached_newer: true,
        staged_newer: false,
    });
    match action {
        LaunchAction::UpdateNow | LaunchAction::StageInBackground | LaunchAction::SwapStaged => {
            let staged = tokio::task::spawn_blocking(crate::update_auto::stage_latest)
                .await
                .ok()?
                .ok()??;
            Some(format!(
                "⚒ Forge {staged} is downloaded — it takes over the next time you start Forge."
            ))
        }
        LaunchAction::UpgradeInBackground(command) => {
            let ok = tokio::task::spawn_blocking(move || run_quietly(&command))
                .await
                .ok()?;
            ok.then(|| {
                format!(
                    "⚒ Forge {version} was installed in the background ({}) — it takes over the next time you start Forge.",
                    method.wire_name()
                )
            })
        }
        LaunchAction::Nothing => None,
    }
}

/// Run a package manager's upgrade with no terminal attached; true when it exited cleanly.
fn run_quietly(command: &str) -> bool {
    let mut cmd = if cfg!(windows) {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", command]);
        c
    } else {
        let mut c = std::process::Command::new("sh");
        c.args(["-c", command]);
        c
    };
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Whether `tag` (e.g. "v0.4.0") is a newer SemVer than `current` ("0.3.0"). Lenient: a malformed
/// tag is treated as not-newer (so we never nag on garbage).
pub(crate) fn is_newer(tag: &str, current: &str) -> bool {
    match (parse_semver(tag), parse_semver(current)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

fn parse_semver(s: &str) -> Option<(u64, u64, u64)> {
    let s = s.trim().trim_start_matches('v');
    // Drop any pre-release/build suffix.
    let core = s.split(['-', '+']).next().unwrap_or(s);
    let mut it = core.split('.');
    let maj = it.next()?.parse().ok()?;
    let min = it.next().unwrap_or("0").parse().ok()?;
    let patch = it.next().unwrap_or("0").parse().ok()?;
    Some((maj, min, patch))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_version_detected_across_components() {
        assert!(is_newer("v0.3.1", "0.3.0"));
        assert!(is_newer("0.4.0", "0.3.9"));
        assert!(is_newer("v1.0.0", "0.99.99"));
        assert!(!is_newer("v0.3.0", "0.3.0"));
        assert!(!is_newer("v0.2.0", "0.3.0"));
    }

    #[test]
    fn lenient_on_garbage_and_partial_tags() {
        assert!(!is_newer("not-a-version", "0.3.0"));
        assert_eq!(parse_semver("v2"), Some((2, 0, 0)));
        assert_eq!(parse_semver("0.3.0-rc.1"), Some((0, 3, 0)));
    }

    #[test]
    fn the_card_offer_follows_the_install_method() {
        let release = Release {
            version: "2.24.0".into(),
            notes: "## Changes\n- Faster\n- Smaller".into(),
            url: "https://example.test/r".into(),
        };
        let own = offer_for(&release, "2.23.6", InstallMethod::Binary);
        assert_eq!(own.plan, UpdatePlan::InPlace);
        assert_eq!(own.notes, vec!["Changes", "Faster", "Smaller"]);
        let brew = offer_for(&release, "2.23.6", InstallMethod::Homebrew);
        assert_eq!(brew.plan, UpdatePlan::Command("brew upgrade forge".into()));
    }
}
