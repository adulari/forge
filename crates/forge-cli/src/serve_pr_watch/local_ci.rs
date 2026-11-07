//! PR Watch's fix turns end in a `git push` the session makes itself, which Forge cannot
//! intercept. What it can do is look at the session's head after a fix turn, run the repository's
//! CI plan on it (docs/features/ci-parity.md) and hand a failing step back to the session instead
//! of waiting minutes for the remote CI to say the same thing. While that check runs, or after it
//! failed on the head the session still has, the watch does not poll or re-arm the merge.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use tokio::sync::mpsc;

use crate::remote::RemoteInput;
use crate::serve::DaemonState;

/// Local-CI fix prompts one fix cycle may send before the watch stops and says so.
const MAX_LOCAL_FAILURES: u32 = 2;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Stage {
    /// A fix prompt went out while the session was at `head`; nothing new to check yet.
    Sent,
    Running,
    Passed,
    Failed,
}

#[derive(Clone, Debug)]
struct Entry {
    head: String,
    phase: Stage,
    failures: u32,
    /// The session's finished-turn count when the check of `head` started. A count that moved
    /// while `head` did not means the session answered the failure prompt without a commit.
    turns: u64,
}

#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    /// Poll as usual.
    Proceed,
    /// Do not poll or merge this tick.
    Hold,
    /// Check this head now (then hold).
    Check(String),
    /// The session keeps failing the local plan, or stopped without fixing it: stop the watch
    /// with this reason.
    GiveUp(String),
}

type Table = Arc<Mutex<HashMap<String, Entry>>>;

fn table() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();
    TABLE.get_or_init(Table::default)
}

fn head_of(root: &Path) -> Option<String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "HEAD"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// A fix prompt just went to the session: remember where its branch stood, so only a commit made
/// after it is checked.
pub(super) fn note_prompt(session: &str, root: &Path) {
    let Some(head) = head_of(root) else { return };
    table().lock().unwrap_or_else(|e| e.into_inner()).insert(
        session.to_string(),
        Entry {
            head,
            phase: Stage::Sent,
            failures: 0,
            turns: 0,
        },
    );
}

fn verdict(entry: Option<&Entry>, fix_turns: u32, head: &str, free: bool, turns: u64) -> Verdict {
    let Some(entry) = entry.filter(|_| fix_turns > 0) else {
        return Verdict::Proceed;
    };
    match entry.phase {
        Stage::Running => Verdict::Hold,
        _ if entry.head != head => {
            if free {
                Verdict::Check(head.to_string())
            } else {
                Verdict::Hold
            }
        }
        Stage::Sent | Stage::Passed => Verdict::Proceed,
        Stage::Failed if entry.failures >= MAX_LOCAL_FAILURES => Verdict::GiveUp(format!(
            "the fix still fails the repository's CI steps locally after {MAX_LOCAL_FAILURES} tries"
        )),
        Stage::Failed if free && turns != entry.turns => Verdict::GiveUp(
            "the fix fails the repository's CI steps locally and the session stopped without a \
             new commit"
                .to_string(),
        ),
        Stage::Failed => Verdict::Hold,
    }
}

fn failure_prompt(text: &str) -> String {
    format!(
        "PR Watch: before waiting for the remote CI, Forge ran this repository's workflow steps \
         locally on your fix and one of them fails.\n\n{text}\n\nFix it, commit, and push with a \
         normal `git push`. Never force-push, never rewrite history. If you cannot fix it, say \
         why and stop."
    )
}

fn settle(session: &str, head: &str, failed: bool) -> Entry {
    let mut map = table().lock().unwrap_or_else(|e| e.into_inner());
    let entry = map.entry(session.to_string()).or_insert_with(|| Entry {
        head: head.to_string(),
        phase: Stage::Sent,
        failures: 0,
        turns: 0,
    });
    entry.head = head.to_string();
    entry.phase = if failed { Stage::Failed } else { Stage::Passed };
    if failed {
        entry.failures += 1;
    }
    entry.clone()
}

/// True when the watch must skip this tick: a check of the session's new head is running or has
/// just failed. Starts the check when a fix turn left a new commit and the session is idle.
pub(super) async fn holds(
    session: &str,
    root: &Path,
    fix_turns: u32,
    free: bool,
    turns: u64,
    input: &mpsc::Sender<RemoteInput>,
) -> Hold {
    let Some(head) = head_of(root) else {
        return Hold::No;
    };
    let current = table()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(session)
        .cloned();
    match verdict(current.as_ref(), fix_turns, &head, free, turns) {
        Verdict::Proceed => Hold::No,
        Verdict::Hold => Hold::Yes,
        Verdict::GiveUp(why) => Hold::GiveUp(why),
        Verdict::Check(head) => {
            if let Some(entry) = table()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get_mut(session)
            {
                entry.phase = Stage::Running;
                entry.head.clone_from(&head);
                entry.turns = turns;
            }
            let (session, root, input) = (session.to_string(), root.to_path_buf(), input.clone());
            tokio::spawn(check(session, root, head, input));
            Hold::Yes
        }
    }
}

/// Whether the sweep skips this watch's poll. Called after the repository root is known and
/// before `advance`; a session that keeps failing the plan gets its watch blocked with the reason.
pub(super) async fn gate(
    state: &Arc<DaemonState>,
    watch: &super::Watch,
    root: &Path,
    free: bool,
    handle: &crate::cli::commands::run::SessionDriverHandle,
) -> bool {
    let turns = handle.snapshot_rx.borrow().snapshot.turns_finished;
    let input = &handle.input_tx;
    match holds(&watch.session_id, root, watch.fix_turns, free, turns, input).await {
        Hold::No => false,
        Hold::Yes => true,
        Hold::GiveUp(why) => {
            super::registry().update(&watch.session_id, |w| {
                w.state = super::Phase::Blocked;
                w.blocked_reason = Some(why.clone());
            });
            state.registry.notify_fleet();
            true
        }
    }
}

pub(super) enum Hold {
    No,
    Yes,
    /// Stop the watch with this reason: the session keeps failing the local plan, or stopped
    /// without a commit after it failed.
    GiveUp(String),
}

async fn check(session: String, root: PathBuf, head: String, input: mpsc::Sender<RemoteInput>) {
    let outcome = {
        let root = root.clone();
        tokio::task::spawn_blocking(move || crate::ci_run::automatic_check(&root, "push")).await
    };
    match outcome {
        Ok(crate::ci_run::Check::Failed(text)) => {
            settle(&session, &head, true);
            let _ = input
                .send(RemoteInput::Prompt {
                    text: failure_prompt(&text),
                    attachments: None,
                })
                .await;
        }
        Ok(_) => {
            settle(&session, &head, false);
        }
        // The check itself crashed: do not hold the PR hostage to it.
        Err(_) => {
            settle(&session, &head, false);
        }
    }
}

#[cfg(test)]
mod tests;
