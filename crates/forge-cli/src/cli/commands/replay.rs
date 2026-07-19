use anyhow::{Context, Result};
use std::io::IsTerminal;
use std::path::Path;

use crate::*;

pub(crate) fn open_store() -> Result<Store> {
    // The store lives in a stable per-user data dir so usage/budget and session history persist
    // across restarts and don't reset when `forge` is launched from a different directory (the
    // budget is global per FR-5). Fall back to the legacy cwd-local path only if no data dir
    // resolves. `FORGE_DB` overrides both (tests / power users).
    let store = if let Ok(custom) = std::env::var("FORGE_DB") {
        let path = std::path::PathBuf::from(custom);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("creating store directory")?;
        }
        Store::open(&path).context("opening session store")?
    } else if let Some(dir) = forge_config::data_dir() {
        std::fs::create_dir_all(&dir).context("creating data directory")?;
        let db = dir.join("forge.db");
        // One-time migration: if there's no global store yet but a legacy `./.forge/forge.db` exists in
        // this directory, move its history over so the switch doesn't appear to wipe past usage.
        let legacy = Path::new(".forge/forge.db");
        if !db.exists() && legacy.exists() {
            let _ = std::fs::copy(legacy, &db);
        }
        Store::open(&db).context("opening session store")?
    } else {
        std::fs::create_dir_all(".forge").context("creating .forge directory")?;
        Store::open(Path::new(".forge/forge.db")).context("opening session store")?
    };
    let anywhere_enabled = forge_config::load()
        .map(|config| config.anywhere.enabled && config.anywhere.sync)
        .unwrap_or(false);
    store
        .set_sync_journal_enabled(anywhere_enabled)
        .context("configure Anywhere sync journal")?;
    Ok(store)
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

/// `forge fork <session> [--turn N] [--model id] [--rerun]` — the counterfactual: branch a past
/// session BEFORE turn N, holding every earlier turn verbatim as fixed context, and re-ask that
/// one prompt — optionally on a different model. Complements `forge replay --rerun` (which
/// replays the WHOLE history fresh); a fork changes exactly one variable. Conversation-state
/// only: files are not rewound (use `/checkpoint` for filesystem time travel).
pub(crate) async fn fork_cmd(
    session_prefix: &str,
    turn: Option<usize>,
    model: Option<String>,
    rerun: bool,
    mock: bool,
) -> Result<()> {
    let store = open_store()?;
    let id = resolve_session(&store, session_prefix)?;
    let entries = store.load_replay(&id).context("loading session")?;
    let user_turns: Vec<(&forge_store::ReplayEntry, usize)> = entries
        .iter()
        .filter(|e| e.role == forge_types::Role::User)
        .zip(1usize..)
        .collect();
    if user_turns.is_empty() {
        anyhow::bail!("session {} has no user turns to fork at", &id[..8]);
    }
    let n = turn.unwrap_or(user_turns.len());
    let Some((entry, _)) = user_turns.iter().find(|(_, i)| *i == n) else {
        anyhow::bail!(
            "turn {n} does not exist — session {} has {} user turn(s)",
            &id[..8],
            user_turns.len()
        );
    };
    let prompt = entry.content.clone();
    let at_seq = entry.seq;

    let fork_id = store.fork_session(&id, at_seq).context("forking session")?;
    let f8 = &fork_id[..8];
    println!("✓ forked {} before turn {n} → {f8}", &id[..8]);
    println!("  held constant: turns 1..{}", n.saturating_sub(1));
    println!("  re-asking:     {}", replay_preview(&prompt));
    drop(store); // release before a child process (or a rebuilt session) opens the DB

    if rerun {
        let forge_exe = std::env::current_exe().context("resolving the forge binary")?;
        let mut cmd = std::process::Command::new(forge_exe);
        cmd.arg("run").arg("--resume").arg(&fork_id).arg(&prompt);
        if let Some(m) = &model {
            cmd.args(["--model", m]);
        }
        if mock {
            cmd.arg("--mock");
        }
        let status = cmd.status().context("running the forked turn")?;
        if !status.success() {
            anyhow::bail!("the forked turn failed — `forge replay {f8}` for what happened");
        }
        // The counterfactual card: original vs fork, aligned per turn. The shared prefix is
        // identical by construction, so the diff IS the effect of the change.
        let store = open_store()?;
        let original = store.load_replay(&id)?;
        let forked = store.load_replay(&fork_id)?;
        let d = replay::diff(&original, &forked);
        let a8 = &id[..id.len().min(8)];
        print!("\n{}", replay::render_diff(a8, f8, &d));
        print!("\n{}", replay::render_turn_diff(a8, f8, &original, &forked));
    } else {
        let pin = model
            .as_deref()
            .map(|m| format!(" --model {m}"))
            .unwrap_or_default();
        println!("  continue it:   forge chat --resume {f8}");
        println!("  or re-run:     forge run --resume {f8}{pin} \"{prompt}\"");
    }
    Ok(())
}

/// `forge tree` — the fork lineage: every fork family (source + its counterfactual branches),
/// labeled by first prompt. Sessions with no fork relation are left out — this is the branch
/// view, `forge sessions` is the flat list.
pub(crate) fn tree_cmd() -> Result<()> {
    let store = open_store()?;
    let nodes = store.fork_nodes().context("loading sessions")?;
    let in_family: std::collections::HashSet<&str> = nodes
        .iter()
        .filter_map(|node| node.forked_from.as_deref())
        .chain(
            nodes
                .iter()
                .filter(|node| node.forked_from.is_some())
                .map(|node| node.id.as_str()),
        )
        .collect();
    if in_family.is_empty() {
        println!("no forks yet — `forge fork <session> --turn N [--model id] --rerun`");
        return Ok(());
    }
    let label = |id: &str| -> String {
        let first = store
            .load_replay(id)
            .ok()
            .and_then(|e| replay::user_prompts(&e).into_iter().next())
            .unwrap_or_default();
        replay_preview(&first)
    };
    println!("fork tree — counterfactual branches\n");
    for node in &nodes {
        if node.forked_from.is_some() || !in_family.contains(node.id.as_str()) {
            continue; // roots only here; forks render nested below their source
        }
        println!("● {}  {}", &node.id[..8], label(&node.id));
        for fork in &nodes {
            if fork.forked_from.as_deref() == Some(node.id.as_str()) {
                let at = fork
                    .forked_at_seq
                    .map(|seq| format!(" @seq {seq}"))
                    .unwrap_or_default();
                println!("└─ {}{}  {}", &fork.id[..8], at, label(&fork.id));
            }
        }
    }
    println!("\ncompare any pair: forge replay <a> <b>");
    Ok(())
}

/// First line of a prompt, truncated for one-line tree/fork labels.
fn replay_preview(text: &str) -> String {
    let line = text.lines().next().unwrap_or("");
    let mut out: String = line.chars().take(72).collect();
    if line.chars().count() > 72 {
        out.push('…');
    }
    out
}
