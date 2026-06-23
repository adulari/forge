use anyhow::{Context, Result};
use std::io::IsTerminal;
use std::path::Path;

use crate::*;

pub(crate) fn open_store() -> Result<Store> {
    // The store lives in a stable per-user data dir so usage/budget and session history persist
    // across restarts and don't reset when `forge` is launched from a different directory (the
    // budget is global per FR-5). Fall back to the legacy cwd-local path only if no data dir
    // resolves. `FORGE_DB` overrides both (tests / power users).
    if let Ok(custom) = std::env::var("FORGE_DB") {
        let path = std::path::PathBuf::from(custom);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("creating store directory")?;
        }
        return Store::open(&path).context("opening session store");
    }
    let Some(dir) = forge_config::data_dir() else {
        std::fs::create_dir_all(".forge").context("creating .forge directory")?;
        return Store::open(Path::new(".forge/forge.db")).context("opening session store");
    };
    std::fs::create_dir_all(&dir).context("creating data directory")?;
    let db = dir.join("forge.db");
    // One-time migration: if there's no global store yet but a legacy `./.forge/forge.db` exists in
    // this directory, move its history over so the switch doesn't appear to wipe past usage.
    let legacy = Path::new(".forge/forge.db");
    if !db.exists() && legacy.exists() {
        let _ = std::fs::copy(legacy, &db);
    }
    Store::open(&db).context("opening session store")
}

/// How `forge chat` should handle session continuity on startup.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ResumeMode {
    /// Start a brand-new session (default — no flags given).
    Fresh,
    /// Reattach to a specific, already-resolved full session id.
    Id(String),
    /// Open the interactive session picker on the first TUI frame.
    Picker,
}

/// Resolve the `--continue` / `--resume` flags into a [`ResumeMode`].
///
/// * `--continue`         → `Id(most_recent)` or a clean error when there are no sessions
/// * `--resume <prefix>`  → resolve prefix → `Id`
/// * `--resume` (bare)    → `Picker` (headless: bail with a clear message)
/// * neither              → `Fresh`
pub(crate) fn resolve_resume_mode(
    do_continue: bool,
    resume: Option<Option<String>>,
    store: &Store,
    plain: bool,
) -> Result<ResumeMode> {
    match (do_continue, resume) {
        (true, _) => {
            let id = store
                .most_recent_session_id()
                .context("looking up most-recent session")?
                .ok_or_else(|| {
                    anyhow::anyhow!("no prior sessions — run `forge chat` to start one")
                })?;
            Ok(ResumeMode::Id(id))
        }
        (false, Some(Some(prefix))) => {
            let id = resolve_session(store, &prefix)?;
            Ok(ResumeMode::Id(id))
        }
        (false, Some(None)) => {
            if plain || !std::io::stdout().is_terminal() {
                anyhow::bail!(
                    "bare --resume requires the interactive TUI; use `--resume <id>` in plain/headless mode"
                );
            }
            Ok(ResumeMode::Picker)
        }
        (false, None) => Ok(ResumeMode::Fresh),
    }
}

/// Resolve a (possibly abbreviated) session id to a single full id, git-style.
pub(crate) fn resolve_session(store: &Store, prefix: &str) -> Result<String> {
    let mut matches = store
        .matching_session_ids(prefix)
        .context("looking up session")?;
    match matches.len() {
        0 => anyhow::bail!("no session matching '{prefix}' — see `forge sessions`"),
        1 => Ok(matches.remove(0)),
        n => anyhow::bail!("'{prefix}' is ambiguous ({n} sessions match) — use more characters"),
    }
}

pub(crate) fn sessions() -> Result<()> {
    let store = open_store()?;
    let list = store.list_sessions().context("listing sessions")?;
    if list.is_empty() {
        println!("no sessions yet — run `forge run \"<task>\"` to start one");
        return Ok(());
    }
    for s in list {
        let id: String = s.id.chars().take(8).collect();
        println!(
            "{id}  {:>7}  {:>3} msgs  ${:>8.4}  {}",
            fmt_age(s.last_activity),
            s.message_count,
            s.total_cost_usd,
            session_title(s.preview.as_deref()),
        );
    }
    Ok(())
}

/// `forge replay <id>` reconstructs a session's transcript; `forge replay <a> <b>` diffs two.
pub(crate) fn replay_cmd(ids: &[String], json: bool) -> Result<()> {
    let store = open_store()?;
    let resolve = |prefix: &str| -> Result<String> {
        let mut matches = store
            .matching_session_ids(prefix)
            .with_context(|| format!("resolving session {prefix}"))?;
        match matches.len() {
            0 => anyhow::bail!("no session matches '{prefix}' — see `forge sessions`"),
            1 => Ok(matches.remove(0)),
            n => anyhow::bail!("'{prefix}' is ambiguous ({n} sessions) — use more characters"),
        }
    };
    match ids {
        [one] => {
            let id = resolve(one)?;
            let entries = store.load_replay(&id).context("loading replay")?;
            if entries.is_empty() {
                if json {
                    println!(
                        "{{\"session_id\":\"{}\",\"turns\":[]}}",
                        &id[..id.len().min(8)]
                    );
                } else {
                    println!("session {} has no messages", &id[..id.len().min(8)]);
                }
                return Ok(());
            }
            if json {
                println!("{}", replay::render_json(&id, &entries));
            } else {
                print!(
                    "{}",
                    replay::render_transcript(&id[..id.len().min(8)], &entries)
                );
            }
        }
        [a, b] => {
            if json {
                anyhow::bail!("--json is only valid with a single session id");
            }
            let (ida, idb) = (resolve(a)?, resolve(b)?);
            let ea = store.load_replay(&ida).context("loading replay a")?;
            let eb = store.load_replay(&idb).context("loading replay b")?;
            let d = replay::diff(&ea, &eb);
            let fa8 = &ida[..ida.len().min(8)];
            let fb8 = &idb[..idb.len().min(8)];
            print!("{}", replay::render_diff(fa8, fb8, &d));
            print!("\n{}", replay::render_turn_diff(fa8, fb8, &ea, &eb));
        }
        _ => anyhow::bail!("usage: forge replay <id> [<id-to-diff-against>]"),
    }
    Ok(())
}

/// `forge replay <id> --rerun` — re-execute a past session's user prompts on the CURRENT
/// model/mesh in a fresh session, then diff the new run against the original. This is the
/// "true model re-execution" half of session replay (the rest is read-only reconstruction):
/// it answers "would today's model/config solve this the same way?" — auditable and
/// reproducible. Tools run under the normal permission mode, exactly as `forge run` does, so a
/// re-run is no more privileged than re-typing the prompts yourself.
pub(crate) async fn replay_rerun_cmd(ids: &[String]) -> Result<()> {
    let [one] = ids else {
        anyhow::bail!("--rerun takes exactly one session id");
    };
    let store = open_store()?;
    let mut matches = store
        .matching_session_ids(one)
        .with_context(|| format!("resolving session {one}"))?;
    let id = match matches.len() {
        0 => anyhow::bail!("no session matches '{one}' — see `forge sessions`"),
        1 => matches.remove(0),
        n => anyhow::bail!("'{one}' is ambiguous ({n} sessions) — use more characters"),
    };
    let original = store.load_replay(&id).context("loading original session")?;
    let prompts = replay::user_prompts(&original);
    if prompts.is_empty() {
        anyhow::bail!(
            "session {} has no user prompts to re-run",
            &id[..id.len().min(8)]
        );
    }

    // Re-run under the user's configured permission mode (mock=false): tools are gated exactly
    // as a normal `forge run`, so re-execution is no more privileged than re-typing the prompts.
    let mut session = build_session(false, None, false, None, None)
        .await
        .context("building the re-run session")?;
    let new_id = session.session_id().to_string();
    eprintln!(
        "re-running {} prompt(s) from {} into fresh session {} …\n",
        prompts.len(),
        &id[..id.len().min(8)],
        &new_id[..new_id.len().min(8)]
    );
    for (i, prompt) in prompts.iter().enumerate() {
        eprintln!("── re-run turn {}/{} ──", i + 1, prompts.len());
        session
            .run_turn(prompt)
            .await
            .with_context(|| format!("re-running turn {}", i + 1))?;
    }
    drop(session); // release the session's store handle before we read the new record back

    let store = open_store()?;
    let replayed = store
        .load_replay(&new_id)
        .context("loading the re-run session")?;
    let d = replay::diff(&original, &replayed);
    let fa = &id[..id.len().min(8)];
    let fb = &new_id[..new_id.len().min(8)];
    print!("\n{}", replay::render_diff(fa, fb, &d));
    print!(
        "\n{}",
        replay::render_turn_diff(fa, fb, &original, &replayed)
    );
    Ok(())
}
