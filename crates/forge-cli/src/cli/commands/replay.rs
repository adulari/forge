use anyhow::{Context, Result};
use std::io::IsTerminal;
use std::io::Write as _;
use std::path::Path;

use crate::*;

/// Where a `cfg(test)` build keeps its store.
///
/// Unit tests reach `open_store` through the ordinary session builders, so without this they open
/// the developer's REAL global store — and any test run from a branch carrying a new migration
/// upgrades it in place. That has bitten twice: locally it crash-looped `forge-serve.service` and
/// killed the Anywhere connector, and on the self-hosted runner it broke an unrelated PR, because
/// one branch's test run migrated the shared store past what the other branch's build supports.
///
/// Per-test rather than per-process. `open_store` takes no arguments, so the test's identity has
/// to come from somewhere: libtest names each test's thread after the test itself, which gives a
/// stable key without an env var concurrent tests would race on. One shared store per process was
/// tried first and produced `database is locked` under the parallel harness — tests writing to one
/// SQLite file contend on the immediate transactions the store takes.
#[cfg(test)]
fn test_store_path() -> std::path::PathBuf {
    let key = std::thread::current()
        .name()
        .map(|name| {
            name.chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                .collect::<String>()
        })
        .unwrap_or_else(|| "unnamed".to_string());
    let dir = std::env::temp_dir().join(format!("forge-test-store-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    dir.join(format!("{key}.db"))
}

/// Which file in the data dir is "the store" for THIS build.
///
/// A debug build gets its own `forge-dev.db`. Four times now, something built from a working tree
/// has opened the real `forge.db` and migrated it to whatever schema that branch carried; the
/// installed release binary then refuses the store (`SchemaTooNew`, correctly) and `forge serve`
/// crash-loops — most recently 632 consecutive restarts with Anywhere down throughout. The
/// per-caller fixes (scrubbing the env from the quota probe, pointing MCP configs elsewhere) each
/// close one route; this closes the class, because `cargo run`, `target/debug/forge`, a dev-built
/// MCP agent and a test that forgot to set the env all resolve here.
///
/// `FORGE_DB` still overrides everything, so deliberately debugging against real data is unchanged.
fn default_store_file_name() -> &'static str {
    if cfg!(debug_assertions) {
        "forge-dev.db"
    } else {
        "forge.db"
    }
}

/// The file [`open_store`] opens, canonicalized when it exists. The daemon records it in
/// `serve-state.json` so a client only hands work to a daemon that owns the same store.
pub(crate) fn resolved_store_path() -> std::path::PathBuf {
    let path = match std::env::var("FORGE_DB") {
        Ok(custom) => std::path::PathBuf::from(custom),
        Err(_) => forge_config::data_dir()
            .map(|dir| dir.join(default_store_file_name()))
            .unwrap_or_else(|| std::path::PathBuf::from(".forge/forge.db")),
    };
    std::fs::canonicalize(&path).unwrap_or(path)
}

/// The data dir holds the session store and kept tool output: owner-only (0700), like the config
/// dir that holds the secrets.
#[cfg(not(test))]
fn restrict_data_dir(dir: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(dir) {
            if meta.permissions().mode() & 0o077 != 0 {
                let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
            }
        }
    }
    #[cfg(not(unix))]
    let _ = dir;
}

/// The file `open_store` resolves to, without opening it: the same `FORGE_DB`, data-dir and
/// legacy-fallback order.
pub(crate) fn store_path() -> std::path::PathBuf {
    if let Ok(custom) = std::env::var("FORGE_DB") {
        return std::path::PathBuf::from(custom);
    }
    #[cfg(test)]
    return test_store_path();
    #[cfg(not(test))]
    match forge_config::data_dir() {
        Some(dir) => dir.join(default_store_file_name()),
        None => std::path::PathBuf::from(".forge/forge.db"),
    }
}

pub(crate) fn open_store() -> Result<Store> {
    if SHARE_STORE_HANDLES.load(std::sync::atomic::Ordering::Relaxed) {
        open_store_with(|path| Store::open_shared(path))
    } else {
        open_store_with(|path| Store::open(path))
    }
}

/// Set once by `forge chat`: one terminal session wants the same file for the resume lookup, the
/// presence guard, the session and the pricing table within a few hundred milliseconds, and every
/// one of those used to open, migrate and pool it again. From here on, handles opened while
/// another is alive reuse its connections ([`Store::open_shared`]). The daemon hosts many
/// sessions and keeps opening independent handles.
static SHARE_STORE_HANDLES: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

pub(crate) fn share_store_handles() {
    SHARE_STORE_HANDLES.store(true, std::sync::atomic::Ordering::Relaxed);
}

fn open_store_with(
    open: impl Fn(&Path) -> std::result::Result<Store, forge_store::StoreError>,
) -> Result<Store> {
    let _opened = crate::startup_trace::Span::new("open_store");
    // The store lives in a stable per-user data dir so usage/budget and session history persist
    // across restarts and don't reset when `forge` is launched from a different directory (the
    // budget is global per FR-5). Fall back to the legacy cwd-local path only if no data dir
    // resolves. `FORGE_DB` overrides both (tests / power users).
    #[cfg(test)]
    let store = {
        // An explicit FORGE_DB still wins, so a test that wants a specific store keeps control.
        let path = std::env::var("FORGE_DB")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| test_store_path());
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("creating store directory")?;
        }
        open(&path).context("opening session store")?
    };
    #[cfg(not(test))]
    let store = if let Ok(custom) = std::env::var("FORGE_DB") {
        let path = std::path::PathBuf::from(custom);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("creating store directory")?;
        }
        open(&path).context("opening session store")?
    } else if let Some(dir) = forge_config::data_dir() {
        std::fs::create_dir_all(&dir).context("creating data directory")?;
        restrict_data_dir(&dir);
        let db = dir.join(default_store_file_name());
        // One-time migration: if there's no global store yet but a legacy `./.forge/forge.db` exists in
        // this directory, move its history over so the switch doesn't appear to wipe past usage.
        let legacy = Path::new(".forge/forge.db");
        if !db.exists() && legacy.exists() {
            let _ = std::fs::copy(legacy, &db);
        }
        open(&db).context("opening session store")?
    } else {
        std::fs::create_dir_all(".forge").context("creating .forge directory")?;
        open(Path::new(".forge/forge.db")).context("opening session store")?
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
/// * `--continue`         → `Id(most_recent in this directory)`, or with `--all-dirs` the most
///   recent of the whole store (like `claude -c`, which is per project); a clean error when none
/// * `--resume <prefix>`  → resolve prefix → `Id`
/// * `--resume` (bare)    → `Picker` (headless: bail with a clear message)
/// * neither              → `Fresh`
pub(crate) fn resolve_resume_mode(
    do_continue: bool,
    all_dirs: bool,
    resume: Option<Option<String>>,
    store: &Store,
    plain: bool,
) -> Result<ResumeMode> {
    match (do_continue, resume) {
        (true, _) => {
            let directory = if all_dirs {
                None
            } else {
                Some(current_workspace()?)
            };
            let id = most_recent_to_continue(store, directory.as_deref())?;
            ensure_not_daemon_driven(store, &id)?;
            unarchive_on_resume(store, &id);
            Ok(ResumeMode::Id(id))
        }
        (false, Some(Some(prefix))) => {
            let id = resolve_session(store, &prefix)?;
            ensure_not_daemon_driven(store, &id)?;
            unarchive_on_resume(store, &id);
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

/// The session `--continue` reattaches: the most recent one whose workspace is `directory`, or
/// the most recent of the whole store when `directory` is `None`.
pub(crate) fn most_recent_to_continue(store: &Store, directory: Option<&str>) -> Result<String> {
    let anywhere = store
        .most_recent_session_id()
        .context("looking up most-recent session")?;
    let Some(directory) = directory else {
        return anywhere
            .ok_or_else(|| anyhow::anyhow!("no prior sessions — run `forge chat` to start one"));
    };
    match store
        .most_recent_session_id_in(directory)
        .context("looking up most-recent session in this directory")?
    {
        Some(id) => Ok(id),
        None if anywhere.is_some() => anyhow::bail!(
            "no prior session in {directory} — `--continue` is per directory; add `--all-dirs` \
             for the most recent session anywhere, or pick one with `forge sessions` and \
             `--resume <id>`"
        ),
        None => anyhow::bail!("no prior sessions — run `forge chat` to start one"),
    }
}

/// The current directory in the form a new session records as its workspace (not canonicalised:
/// on Windows that adds a `\\?\` prefix no recorded workspace carries).
fn current_workspace() -> Result<String> {
    Ok(std::env::current_dir()
        .context("resolving the current directory")?
        .display()
        .to_string())
}

/// Resolve a (possibly abbreviated) session id to a single full id, git-style. A value that
/// matches no id is tried as a session name (`forge run --name`); the newest such session wins.
pub(crate) fn resolve_session(store: &Store, prefix: &str) -> Result<String> {
    resolve_session_with(store, prefix, false)
}

/// [`resolve_session`] for irreversible actions: a name shared by several sessions is ambiguous
/// instead of silently picking the newest.
pub(crate) fn resolve_session_exact(store: &Store, prefix: &str) -> Result<String> {
    resolve_session_with(store, prefix, true)
}

fn resolve_session_with(store: &Store, prefix: &str, unique_title: bool) -> Result<String> {
    let mut matches = store
        .matching_session_ids(prefix)
        .context("looking up session")?;
    if matches.is_empty() {
        let titled = store.count_sessions_titled(prefix.trim())?;
        if unique_title && titled > 1 {
            anyhow::bail!(
                "'{prefix}' is ambiguous ({titled} sessions have that name) — use the session id"
            );
        }
        if let Some(id) = store.newest_session_titled(prefix.trim())? {
            return Ok(id);
        }
    }
    match matches.len() {
        0 => anyhow::bail!("no session matching '{prefix}' — see `forge sessions`"),
        1 => Ok(matches.remove(0)),
        n => anyhow::bail!("'{prefix}' is ambiguous ({n} sessions match) — use more characters"),
    }
}

pub(crate) fn sessions(archived_only: bool, all: bool, unarchive: Option<&str>) -> Result<()> {
    let store = open_store()?;
    if let Some(prefix) = unarchive {
        let id = resolve_session(&store, prefix)?;
        if !store.session_archived(&id)? {
            println!("{} is not archived", &id[..id.len().min(8)]);
            return Ok(());
        }
        store.unarchive_session(&id).context("un-archiving")?;
        println!(
            "un-archived {} — it is back in `forge sessions`",
            &id[..id.len().min(8)]
        );
        return Ok(());
    }
    let list: Vec<_> = if archived_only || all {
        store
            .list_sessions_for_resume()
            .context("listing sessions")?
            .into_iter()
            .filter(|s| all || s.archived)
            .collect()
    } else {
        store.list_sessions().context("listing sessions")?
    };
    if list.is_empty() {
        if archived_only {
            println!("no archived sessions");
        } else {
            println!("no sessions yet — run `forge run \"<task>\"` to start one");
        }
        return Ok(());
    }
    // `forge sessions | head` closes the pipe early: stop quietly instead of panicking in println!.
    let mut out = std::io::stdout().lock();
    for s in list {
        let id: String = s.id.chars().take(8).collect();
        let tag = if s.archived { "  [archived]" } else { "" };
        if writeln!(
            out,
            "{id}  {:>7}  {:>3} msgs  ${:>8.4}  {}{tag}",
            fmt_age(s.last_activity),
            s.message_count,
            s.total_cost_usd,
            session_title(
                s.title
                    .as_deref()
                    .filter(|t| !t.trim().is_empty())
                    .or(s.preview.as_deref())
            ),
        )
        .is_err()
        {
            return Ok(());
        }
    }
    drop(out);
    if archived_only {
        println!(
            "\nbring one back: forge sessions --unarchive <id>  (or forge chat --resume <id>)"
        );
    }
    Ok(())
}

/// Refuse a terminal resume of a session the `forge serve` daemon is driving right now: both
/// would append to one session and interleave their turns. The durable `daemon_live` flag alone
/// is not enough (a killed daemon leaves it set), so it only counts while the daemon that wrote
/// `serve-state.json` is still a live process.
pub(crate) fn ensure_not_daemon_driven(store: &Store, id: &str) -> Result<()> {
    if !store.session_daemon_live(id).unwrap_or(false) {
        return Ok(());
    }
    refuse_if_daemon_alive(id, crate::serve::read_state().ok().flatten())
}

fn refuse_if_daemon_alive(id: &str, state: Option<crate::serve::ServeState>) -> Result<()> {
    let Some(state) = state.filter(|s| s.process_is_alive()) else {
        return Ok(());
    };
    anyhow::bail!(
        "session {} is being driven by the forge serve daemon (pid {}); resuming it here would \
         put two writers on one session. Watch or steer it with `forge attach {}`, or stop it \
         from the app first.",
        &id[..id.len().min(8)],
        state.pid,
        &id[..id.len().min(8)]
    )
}

/// Resuming from the terminal is an explicit "bring this back": an archived session comes out of
/// the archive, as it does when the daemon resumes one.
pub(crate) fn unarchive_on_resume(store: &Store, id: &str) {
    if store.session_archived(id).unwrap_or(false) && store.unarchive_session(id).is_ok() {
        eprintln!("un-archived {}", &id[..id.len().min(8)]);
    }
}

/// `forge replay <id>` reconstructs a session's transcript; `forge replay <a> <b>` diffs two;
/// `forge replay <id> --html <path>` renders it to one self-contained HTML file.
pub(crate) fn replay_cmd(
    ids: &[String],
    json: bool,
    html: Option<&Path>,
    redact: bool,
) -> Result<()> {
    let store = open_store()?;
    let redactor = redact.then(forge_core::redact::Redactor::from_environment);
    let scrub = |entries: &mut Vec<forge_store::ReplayEntry>| {
        if let Some(r) = &redactor {
            r.redact_replay(entries);
        }
    };
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
            let mut entries = store.load_replay(&id).context("loading replay")?;
            scrub(&mut entries);
            if entries.is_empty() {
                if let Some(path) = html {
                    let out = replay::render_html(&id[..id.len().min(8)], &entries);
                    std::fs::write(path, out)
                        .with_context(|| format!("writing {}", path.display()))?;
                    println!(
                        "session {} has no messages — wrote an empty transcript to {}",
                        &id[..id.len().min(8)],
                        path.display()
                    );
                } else if json {
                    println!(
                        "{{\"session_id\":\"{}\",\"turns\":[]}}",
                        &id[..id.len().min(8)]
                    );
                } else {
                    println!("session {} has no messages", &id[..id.len().min(8)]);
                }
                return Ok(());
            }
            if let Some(path) = html {
                let out = replay::render_html(&id[..id.len().min(8)], &entries);
                std::fs::write(path, out).with_context(|| format!("writing {}", path.display()))?;
                println!(
                    "✓ wrote {} ({} turns) to {}",
                    &id[..id.len().min(8)],
                    entries.len(),
                    path.display()
                );
            } else if json {
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
            if html.is_some() {
                anyhow::bail!("--html is only valid with a single session id");
            }
            let (ida, idb) = (resolve(a)?, resolve(b)?);
            let mut ea = store.load_replay(&ida).context("loading replay a")?;
            let mut eb = store.load_replay(&idb).context("loading replay b")?;
            scrub(&mut ea);
            scrub(&mut eb);
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
    // Validate before the fork row exists: a bad `--model` used to be reported only after the
    // new session had been written, leaving an orphan behind with a zero exit code.
    if let Some(m) = model.as_deref() {
        super::run::ensure_pin_providers_known(m)?;
    }

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
    let label = |id: &str| -> String {
        let first = store
            .load_replay(id)
            .ok()
            .and_then(|e| replay::user_prompts(&e).into_iter().next())
            .unwrap_or_default();
        replay_preview(&first)
    };
    match render_fork_tree(&nodes, label) {
        Some(tree) => {
            println!("fork tree — counterfactual branches\n");
            print!("{tree}");
            println!("\ncompare any pair: forge replay <a> <b>");
        }
        None => println!("no forks yet — `forge fork <session> --turn N [--model id] --rerun`"),
    }
    Ok(())
}

/// Draw every fork family as a nested tree (`├─` for a non-last sibling, `└─` for the last, a
/// `│` rail under non-last branches), forks of forks included. A fork whose source session no
/// longer exists is drawn as its own root, marked `(source removed)`, so lineage never vanishes.
/// `None` when no session is part of a fork relation.
fn render_fork_tree(
    nodes: &[forge_store::ForkNode],
    label: impl Fn(&str) -> String,
) -> Option<String> {
    use std::collections::{HashMap, HashSet};
    let known: HashSet<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
    let mut children: HashMap<&str, Vec<&forge_store::ForkNode>> = HashMap::new();
    for node in nodes {
        if let Some(parent) = node.forked_from.as_deref() {
            children.entry(parent).or_default().push(node);
        }
    }
    let is_root = |n: &&forge_store::ForkNode| match n.forked_from.as_deref() {
        None => children.contains_key(n.id.as_str()),
        Some(parent) => !known.contains(parent),
    };
    let roots: Vec<&forge_store::ForkNode> = nodes.iter().filter(is_root).collect();
    if roots.is_empty() {
        return None;
    }
    fn short(id: &str) -> &str {
        &id[..id.len().min(8)]
    }
    fn walk(
        out: &mut String,
        node: &forge_store::ForkNode,
        children: &std::collections::HashMap<&str, Vec<&forge_store::ForkNode>>,
        label: &dyn Fn(&str) -> String,
        prefix: &str,
        seen: &mut std::collections::HashSet<String>,
    ) {
        let Some(kids) = children.get(node.id.as_str()) else {
            return;
        };
        for (i, fork) in kids.iter().enumerate() {
            if !seen.insert(fork.id.clone()) {
                continue;
            }
            let last = i + 1 == kids.len();
            let at = fork
                .forked_at_seq
                .map(|seq| format!(" @seq {seq}"))
                .unwrap_or_default();
            out.push_str(&format!(
                "{prefix}{} {}{at}  {}\n",
                if last { "└─" } else { "├─" },
                short(&fork.id),
                label(&fork.id)
            ));
            let next = format!("{prefix}{}", if last { "   " } else { "│  " });
            walk(out, fork, children, label, &next, seen);
        }
    }
    let mut out = String::new();
    let mut seen: HashSet<String> = HashSet::new();
    for root in roots {
        if !seen.insert(root.id.clone()) {
            continue;
        }
        let orphan = if root.forked_from.is_some() {
            "  (source removed)"
        } else {
            ""
        };
        out.push_str(&format!(
            "● {}  {}{orphan}\n",
            short(&root.id),
            label(&root.id)
        ));
        walk(&mut out, root, &children, &label, "", &mut seen);
    }
    Some(out)
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

#[cfg(test)]
mod store_path_tests {
    use super::default_store_file_name;

    /// The whole point of the split: a build with debug assertions — `cargo run`,
    /// `target/debug/forge`, a dev-built MCP agent, a test that forgot `FORGE_DB` — must not
    /// resolve to the release store and migrate it. This test necessarily runs in a debug profile,
    /// so it pins the half that can actually be asserted here, and the release name is pinned as a
    /// literal so renaming it is a deliberate act rather than a typo.
    #[test]
    fn a_debug_build_does_not_resolve_to_the_release_store() {
        assert_eq!(default_store_file_name(), "forge-dev.db");
        assert_ne!(default_store_file_name(), "forge.db");
    }
}

#[cfg(test)]
mod fork_tree_tests {
    use super::render_fork_tree;
    use forge_store::ForkNode;

    fn node(id: &str, from: Option<&str>, at: Option<i64>) -> ForkNode {
        ForkNode {
            id: id.to_string(),
            title: None,
            forked_from: from.map(str::to_string),
            forked_at_seq: at,
            created_at: 0,
        }
    }

    #[test]
    fn forks_of_forks_nest_and_siblings_use_branch_glyphs() {
        let nodes = vec![
            node("sssssssss", None, None),
            node("aaaaaaaaa", Some("sssssssss"), Some(8)),
            node("ccccccccc", Some("sssssssss"), Some(8)),
            node("bbbbbbbbb", Some("aaaaaaaaa"), Some(4)),
        ];
        let tree = render_fork_tree(&nodes, |id| format!("L{}", &id[..1])).unwrap();
        assert_eq!(
            tree,
            "● ssssssss  Ls\n├─ aaaaaaaa @seq 8  La\n│  └─ bbbbbbbb @seq 4  Lb\n└─ cccccccc @seq 8  Lc\n"
        );
    }

    #[test]
    fn a_fork_whose_source_was_deleted_is_still_drawn() {
        let nodes = vec![
            node("aaaaaaaaa", Some("gone"), Some(2)),
            node("bbbbbbbbb", Some("aaaaaaaaa"), Some(6)),
        ];
        let tree = render_fork_tree(&nodes, |_| String::new()).unwrap();
        assert!(tree.contains("● aaaaaaaa"), "{tree}");
        assert!(tree.contains("(source removed)"), "{tree}");
        assert!(tree.contains("└─ bbbbbbbb @seq 6"), "{tree}");
    }

    #[test]
    fn no_fork_relation_means_no_tree() {
        let nodes = vec![node("sssssssss", None, None)];
        assert!(render_fork_tree(&nodes, |_| String::new()).is_none());
    }
}

#[cfg(test)]
mod daemon_guard_tests {
    use super::*;

    #[test]
    fn delete_resolution_refuses_a_name_shared_by_several_sessions() {
        let store = Store::open_in_memory().unwrap();
        let first = store.create_session("/p", "Smith").unwrap();
        store.set_session_title(&first, "fix tests").unwrap();
        assert_eq!(resolve_session_exact(&store, "fix tests").unwrap(), first);
        let second = store.create_session("/p", "Smith").unwrap();
        store.set_session_title(&second, "fix tests").unwrap();
        let err = resolve_session_exact(&store, "fix tests").unwrap_err();
        assert!(err.to_string().contains("ambiguous"), "{err}");
        assert_eq!(resolve_session(&store, "fix tests").unwrap(), second);
    }

    fn state(pid: u32) -> crate::serve::ServeState {
        crate::serve::ServeState {
            pid,
            port: 1,
            exposure: "local".into(),
            base_url: "http://127.0.0.1:1/t".into(),
            token: "t".into(),
            started_at: 0,
            store_path: None,
        }
    }

    #[test]
    fn a_live_daemon_blocks_a_terminal_resume_but_a_dead_one_does_not() {
        let id = "0123456789abcdef";
        let live = refuse_if_daemon_alive(id, Some(state(std::process::id())));
        let message = live.unwrap_err().to_string();
        assert!(message.contains("01234567"), "{message}");
        assert!(message.contains("forge attach"), "{message}");
        // No daemon record, or one whose process is gone (stale flag after a crash): resume works.
        assert!(refuse_if_daemon_alive(id, None).is_ok());
        assert!(refuse_if_daemon_alive(id, Some(state(u32::MAX - 1))).is_ok());
    }

    #[test]
    fn a_session_the_daemon_does_not_own_is_never_refused() {
        let store = Store::open_in_memory().unwrap();
        let id = store.create_session("/tmp", "default").unwrap();
        assert!(ensure_not_daemon_driven(&store, &id).is_ok());
    }

    #[test]
    fn archived_sessions_come_back_when_resumed_from_the_terminal() {
        let store = Store::open_in_memory().unwrap();
        let id = store.create_session("/tmp", "default").unwrap();
        store.archive_session(&id).unwrap();
        unarchive_on_resume(&store, &id);
        assert!(!store.session_archived(&id).unwrap());
    }
}
