//! Isolated `claude` config-dir mirror, so a bridged `claude` subprocess never fires the user's
//! own personal hooks (`~/.claude/settings.json`'s `"hooks"` key, and `settings.local.json`)
//! during a Forge-driven turn. Forge's own tool registry + permission gate must be the ONLY thing
//! that runs a bridged turn's side effects — a globally-installed notification/logging hook firing
//! on every bridged turn is an unwanted leak of the user's personal claude config into a sandboxed
//! Forge session.
//!
//! No CLI flag does this cleanly (verified live against the real `claude` binary): `--bare` skips
//! hooks but forces API-key-only auth (breaks the whole point of the subscription bridge);
//! `--safe-mode` and `--setting-sources ""` both suppress hooks but ALSO blank out the explicit
//! `--mcp-config` server Forge's harness depends on (the model reports zero MCP tools connected).
//!
//! Instead: point the bridged process at an isolated `CLAUDE_CONFIG_DIR` ([`prepare_claude_bridge_home`])
//! that mirrors the real one via symlinks for every entry EXCEPT the hook-bearing settings files,
//! which get a hooks-stripped JSON copy instead. `CLAUDE_CONFIG_DIR` controls where claude reads
//! BOTH its settings/hooks AND its auth/session state from, so symlinking everything else through
//! (`.credentials.json`, `projects/`, `sessions/`, `history.jsonl`, `plugins/`, `cache/`, …) keeps
//! auth, session resume, and prompt-cache continuity fully intact.

use std::path::Path;

/// Settings files that may carry a `"hooks"` key.
const FILTERED_SETTINGS_FILES: &[&str] = &["settings.json", "settings.local.json"];

/// Entries safe and useful to copy when Windows cannot create a symlink. Keep this deliberately
/// narrow: unknown files in a user's Claude home may contain credentials, session material, or
/// unrelated application state and must fail closed rather than being copied by default.
#[cfg(any(windows, test))]
const WINDOWS_COPY_ALLOWLIST: &[&str] = &["projects", "history.jsonl"];

/// Top-level JSON keys stripped from a filtered settings file. `hooks` is the primary target;
/// `enabledPlugins`/`extraKnownMarketplaces` are defense-in-depth against a plugin registering its
/// own hooks — plugins are a secondary concern here, hooks is the one that must never fire.
const STRIPPED_KEYS: &[&str] = &["hooks", "enabledPlugins", "extraKnownMarketplaces"];

/// The REAL claude config dir to mirror: `$CLAUDE_CONFIG_DIR` if the user already has one set
/// (respect it rather than silently ignoring it), else `<home>/.claude`.
pub fn real_claude_config_dir() -> Option<std::path::PathBuf> {
    let inherited = std::env::var_os("CLAUDE_CONFIG_DIR").map(std::path::PathBuf::from);
    let bridge_home = forge_config::config_dir().map(|base| base.join(BRIDGE_HOME_DIR_NAME));
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    resolve_real_config_dir(
        inherited,
        bridge_home.as_deref(),
        home.map(std::path::PathBuf::from),
    )
}

/// Directory name of Forge's isolated mirror under the Forge config dir.
pub const BRIDGE_HOME_DIR_NAME: &str = "claude-bridge-home";

/// The pure core of [`real_claude_config_dir`]. An inherited `CLAUDE_CONFIG_DIR` is honoured —
/// UNLESS it is Forge's own isolated mirror. A Forge process launched from inside a bridged
/// `claude` (its `forge mcp-serve` MCP server, and any child session that spawns) inherits the
/// bridge's `CLAUDE_CONFIG_DIR=<bridge home>`; mirroring that dir onto itself replaces every
/// entry with a symlink to itself, and every claude bridge on the machine is "Not logged in"
/// until a spawn with a clean environment rebuilds it (observed 2026-09-02: `.credentials.json ->
/// <bridge home>/.credentials.json`, rebuilt 41 s before opus was excluded for 30 minutes).
fn resolve_real_config_dir(
    inherited: Option<std::path::PathBuf>,
    bridge_home: Option<&Path>,
    home: Option<std::path::PathBuf>,
) -> Option<std::path::PathBuf> {
    if let Some(dir) = inherited {
        let is_own_mirror = bridge_home.is_some_and(|b| same_path(b, &dir));
        if !is_own_mirror {
            return Some(dir);
        }
    }
    Some(home?.join(".claude"))
}

/// Path equality that survives symlinks and trailing slashes when both sides exist.
fn same_path(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Build (or refresh) `isolated_dir` as a mirror of `real_home` (the user's real claude config
/// dir) via symlinks for everything EXCEPT the hook-bearing settings files, which get a filtered
/// copy instead (hooks stripped). Pointing a bridged `claude` subprocess at `isolated_dir` via
/// `CLAUDE_CONFIG_DIR` keeps auth/session/resume continuity fully intact while guaranteeing the
/// user's own hooks never fire during a Forge-driven turn.
///
/// Builds into a private temp sibling directory first, then swaps it into place with renames — a
/// crash mid-build never leaves a half-populated or half-stale dir live.
///
/// # Concurrency (the defect this shape exists for)
///
/// This runs before EVERY claude bridge spawn, and one `forge serve` daemon spawns many bridges at
/// once. The original shape did `remove_dir_all(isolated_dir)` and then repopulated it, so with
/// several sessions running, one spawn's rebuild routinely deleted the live `CLAUDE_CONFIG_DIR`
/// out from under another spawn's already-running `claude`. That child then found no
/// `.credentials.json`, reported that it was not logged in, and Forge — correctly reading its
/// stderr — classified a perfectly healthy subscription as an authentication failure and benched
/// the entire provider. Measured: 303 observations of a credential-less config dir across 4
/// concurrent builders (`concurrent_builds_never_expose_a_credential_less_config_dir`).
///
/// Three properties keep that from recurring:
/// 1. [`BUILD_LOCK`] serializes builds within the process, so builders never share temp state.
/// 2. [`mirror_is_current`] makes the steady state a pure read — the overwhelmingly common case
///    does not touch `isolated_dir` at all, so there is nothing to race with.
/// 3. When a rebuild IS needed, the live dir is renamed aside and the new one renamed into place;
///    the gap is two rename syscalls rather than a full delete-and-repopulate.
///
/// `real_home` not existing (nothing to mirror) is a no-op, not an error — claude then falls back
/// to its own default/unauthenticated behavior, which is an existing-behavior edge case, not a
/// regression introduced by this isolation.
pub fn prepare_claude_bridge_home(real_home: &Path, isolated_dir: &Path) -> anyhow::Result<()> {
    if !real_home.is_dir() {
        return Ok(());
    }
    if same_path(real_home, isolated_dir) {
        anyhow::bail!(
            "refusing to mirror the claude bridge home onto itself ({}); CLAUDE_CONFIG_DIR was \
             inherited from a bridged claude",
            isolated_dir.display()
        );
    }
    let _guard = BUILD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if mirror_is_current(real_home, isolated_dir) {
        return Ok(());
    }
    let parent = isolated_dir
        .parent()
        .ok_or_else(|| anyhow::anyhow!("isolated claude bridge home has no parent directory"))?;
    std::fs::create_dir_all(parent)?;

    // Build fresh into a private temp dir beside the target, then swap it in with renames. The
    // suffix is unique per call (not just per process): two builders in one process must never
    // share a scratch directory, or one deletes the other's half-built mirror.
    let tmp_dir = parent.join(format!(
        ".{}.tmp-{}",
        isolated_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("claude-bridge-home"),
        unique_suffix()
    ));
    if tmp_dir.exists() {
        std::fs::remove_dir_all(&tmp_dir)?;
    }
    std::fs::create_dir_all(&tmp_dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp_dir, std::fs::Permissions::from_mode(0o700))?;
    }

    for entry in std::fs::read_dir(real_home)? {
        let entry = entry?;
        let name = entry.file_name();
        let real_path = entry.path();
        let dest_path = tmp_dir.join(&name);
        let is_filtered_settings = name
            .to_str()
            .is_some_and(|n| FILTERED_SETTINGS_FILES.contains(&n));

        if is_filtered_settings {
            // Hook isolation remains the safety invariant, so a malformed settings file must not
            // abort the bridge-home build. Do report the file-level cause: silently dropping a
            // user's settings makes bridge behavior appear random and is hard to diagnose.
            match filtered_settings_json(&real_path) {
                Ok(Some(filtered)) => std::fs::write(&dest_path, filtered)?,
                Ok(None) => {}
                Err(error) => tracing::warn!(
                    path = %real_path.display(),
                    error = %error,
                    "claude bridge: skipped malformed settings file"
                ),
            }
            continue;
        }

        symlink_through(&real_path, &dest_path)?;
    }

    let retired = isolated_dir.exists().then(|| {
        parent.join(format!(
            ".{}.retired-{}",
            isolated_dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("claude-bridge-home"),
            unique_suffix()
        ))
    });
    if let Some(retired) = &retired {
        std::fs::rename(isolated_dir, retired)?;
    }
    if let Err(error) = std::fs::rename(&tmp_dir, isolated_dir) {
        // Put the previous mirror back rather than leaving the bridge with no config dir at all —
        // a stale mirror still authenticates, an absent one looks exactly like a logged-out CLI.
        if let Some(retired) = &retired {
            let _ = std::fs::rename(retired, isolated_dir);
        }
        let _ = std::fs::remove_dir_all(&tmp_dir);
        return Err(error.into());
    }
    if let Some(retired) = &retired {
        let _ = std::fs::remove_dir_all(retired);
    }
    Ok(())
}

/// Serializes mirror builds within this process. Concurrent bridge spawns are the norm under
/// `forge serve`; without this they interleave scratch-directory and swap steps.
static BUILD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A suffix unique to one call, so two builders never collide on a scratch/retired path.
fn unique_suffix() -> String {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    format!(
        "{}-{}",
        std::process::id(),
        N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}

/// Whether `isolated_dir` already mirrors `real_home` exactly, so the rebuild can be skipped.
///
/// This is what makes the steady state race-free: a bridge spawn while other bridges are running
/// must not disturb the config dir they are reading. Conservative — any doubt returns `false` and
/// the caller rebuilds.
fn mirror_is_current(real_home: &Path, isolated_dir: &Path) -> bool {
    let Ok(real_entries) = std::fs::read_dir(real_home) else {
        return false;
    };
    let mut expected = std::collections::BTreeSet::new();
    for entry in real_entries {
        let Ok(entry) = entry else { return false };
        let name = entry.file_name();
        let dest = isolated_dir.join(&name);
        let is_filtered_settings = name
            .to_str()
            .is_some_and(|n| FILTERED_SETTINGS_FILES.contains(&n));
        if is_filtered_settings {
            match filtered_settings_json(&entry.path()) {
                // A settings file that produces no filtered copy legitimately has no mirror entry.
                Ok(None) => continue,
                Ok(Some(filtered)) => {
                    if std::fs::read(&dest).ok() != Some(filtered) {
                        return false;
                    }
                }
                Err(_) => return false,
            }
        } else if std::fs::read_link(&dest).ok().as_deref() != Some(&entry.path()) {
            return false;
        }
        expected.insert(name);
    }
    // A leftover entry for a file the user has since deleted means the mirror is stale.
    let Ok(mirrored) = std::fs::read_dir(isolated_dir) else {
        return false;
    };
    let mut seen = 0usize;
    for entry in mirrored {
        let Ok(entry) = entry else { return false };
        if !expected.contains(&entry.file_name()) {
            return false;
        }
        seen += 1;
    }
    seen == expected.len()
}

/// Parse `path` as JSON and strip [`STRIPPED_KEYS`]. Missing files are a race-safe no-op; read,
/// parse, and serialization failures are returned so the caller can report the exact file.
fn filtered_settings_json(path: &Path) -> anyhow::Result<Option<Vec<u8>>> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut value: serde_json::Value = serde_json::from_str(&text)?;
    if let Some(obj) = value.as_object_mut() {
        for key in STRIPPED_KEYS {
            obj.remove(*key);
        }
    }
    Ok(Some(serde_json::to_vec_pretty(&value)?))
}

#[cfg(any(windows, test))]
fn is_allowed_windows_copy(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_string();
    WINDOWS_COPY_ALLOWLIST
        .iter()
        .any(|allowed| name.eq_ignore_ascii_case(allowed))
}

#[cfg(unix)]
fn symlink_through(real_path: &Path, dest_path: &Path) -> anyhow::Result<()> {
    std::os::unix::fs::symlink(real_path, dest_path)?;
    Ok(())
}

/// Windows symlinks require Developer Mode or an elevated process — not something Forge can
/// assume. Best-effort: a failed symlink here only means THAT ONE entry (e.g. `projects/` history)
/// isn't mirrored into the isolated dir, so resume-continuity for it is lost on that install; it
/// does not fail the bridge-home build or the turn, and the primary goal (hooks never fire) still
/// holds regardless, since the settings files always go through the filtered-copy branch above.
#[cfg(windows)]
fn symlink_through(real_path: &Path, dest_path: &Path) -> anyhow::Result<()> {
    let result = if real_path.is_dir() {
        std::os::windows::fs::symlink_dir(real_path, dest_path)
    } else {
        std::os::windows::fs::symlink_file(real_path, dest_path)
    };
    if let Err(e) = result {
        if !is_allowed_windows_copy(real_path) {
            tracing::warn!(
                path = %real_path.display(),
                error = %e,
                "claude bridge home: skipped unallowlisted entry because Windows symlinks are unavailable"
            );
        } else if let Err(copy_error) = copy_allowlisted_entry(real_path, dest_path) {
            tracing::warn!(
                path = %real_path.display(),
                symlink_error = %e,
                copy_error = %copy_error,
                "claude bridge home: could not mirror allowlisted entry on Windows"
            );
        } else {
            tracing::warn!(
                path = %real_path.display(),
                error = %e,
                "claude bridge home: symlink unavailable; copied allowlisted entry for continuity"
            );
        }
    }
    Ok(())
}

#[cfg(windows)]
fn copy_allowlisted_entry(real_path: &Path, dest_path: &Path) -> anyhow::Result<()> {
    if real_path.is_dir() {
        std::fs::create_dir_all(dest_path)?;
        for entry in std::fs::read_dir(real_path)? {
            let entry = entry?;
            let source = entry.path();
            copy_allowlisted_entry(&source, &dest_path.join(entry.file_name()))?;
        }
    } else {
        if let Some(parent) = dest_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(real_path, dest_path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_json(path: &Path, value: &serde_json::Value) {
        std::fs::write(path, serde_json::to_string_pretty(value).unwrap()).unwrap();
    }

    #[test]
    fn an_inherited_bridge_home_is_not_treated_as_the_real_config_dir() {
        let root = tempfile::tempdir().unwrap();
        let bridge = root.path().join("forge").join(BRIDGE_HOME_DIR_NAME);
        std::fs::create_dir_all(&bridge).unwrap();
        let home = root.path().join("home");
        let resolved =
            resolve_real_config_dir(Some(bridge.clone()), Some(&bridge), Some(home.clone()))
                .unwrap();
        assert_eq!(
            resolved,
            home.join(".claude"),
            "falls back to the real home"
        );
        // A trailing slash or a different spelling of the same dir is still the mirror.
        let spelled = root.path().join("forge").join("./claude-bridge-home/");
        let resolved =
            resolve_real_config_dir(Some(spelled), Some(&bridge), Some(home.clone())).unwrap();
        assert_eq!(resolved, home.join(".claude"));
        // A user's own CLAUDE_CONFIG_DIR elsewhere is still honoured.
        let theirs = root.path().join("their-claude");
        let resolved =
            resolve_real_config_dir(Some(theirs.clone()), Some(&bridge), Some(home)).unwrap();
        assert_eq!(resolved, theirs);
    }

    #[test]
    fn prepare_refuses_to_mirror_the_bridge_home_onto_itself() {
        let root = tempfile::tempdir().unwrap();
        let bridge = root.path().join(BRIDGE_HOME_DIR_NAME);
        std::fs::create_dir_all(&bridge).unwrap();
        std::fs::write(bridge.join(".credentials.json"), "{}").unwrap();
        let err = prepare_claude_bridge_home(&bridge, &bridge).unwrap_err();
        assert!(err.to_string().contains("onto itself"), "{err}");
        let meta = std::fs::symlink_metadata(bridge.join(".credentials.json")).unwrap();
        assert!(
            meta.is_file(),
            "the live credentials file is untouched, not a self-link"
        );
    }

    #[test]
    fn hooks_key_is_stripped_but_other_keys_survive() {
        let real = tempfile::tempdir().unwrap();
        write_json(
            &real.path().join("settings.json"),
            &serde_json::json!({
                "hooks": {"PostToolUse": [{"matcher": "*", "hooks": []}]},
                "theme": "dark",
                "enabledPlugins": {"foo": true},
            }),
        );
        let isolated = real.path().parent().unwrap().join("isolated-hooks");
        prepare_claude_bridge_home(real.path(), &isolated).unwrap();

        let out: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(isolated.join("settings.json")).unwrap())
                .unwrap();
        assert!(out.get("hooks").is_none(), "hooks must be stripped");
        assert!(
            out.get("enabledPlugins").is_none(),
            "enabledPlugins must be stripped"
        );
        assert_eq!(out["theme"], "dark", "unrelated keys survive");

        let _ = std::fs::remove_dir_all(&isolated);
    }

    #[test]
    fn non_settings_entries_are_symlinked_through_unchanged() {
        let real = tempfile::tempdir().unwrap();
        std::fs::write(real.path().join(".credentials.json"), r#"{"token":"abc"}"#).unwrap();
        std::fs::create_dir_all(real.path().join("projects")).unwrap();
        std::fs::write(real.path().join("projects/session.jsonl"), "line1\n").unwrap();

        let isolated = real.path().parent().unwrap().join("isolated-symlinks");
        prepare_claude_bridge_home(real.path(), &isolated).unwrap();

        assert_eq!(
            std::fs::read_to_string(isolated.join(".credentials.json")).unwrap(),
            r#"{"token":"abc"}"#
        );
        assert_eq!(
            std::fs::read_to_string(isolated.join("projects/session.jsonl")).unwrap(),
            "line1\n"
        );
        #[cfg(unix)]
        {
            let meta = std::fs::symlink_metadata(isolated.join(".credentials.json")).unwrap();
            assert!(
                meta.file_type().is_symlink(),
                "non-settings entries are symlinked, not copied"
            );
        }

        let _ = std::fs::remove_dir_all(&isolated);
    }

    /// Concurrent bridge spawns must never expose a `CLAUDE_CONFIG_DIR` without credentials in it.
    ///
    /// This is the whole defect in one test. Before the fix this observed a credential-less config
    /// dir 303 times across 4 concurrent builders: a `claude` launched into that window reports it
    /// is not logged in, and Forge classifies a healthy subscription as an auth failure.
    #[test]
    fn concurrent_builds_never_expose_a_credential_less_config_dir() {
        let real = tempfile::tempdir().unwrap();
        std::fs::write(real.path().join(".credentials.json"), r#"{"token":"abc"}"#).unwrap();
        write_json(
            &real.path().join("settings.json"),
            &serde_json::json!({"theme": "dark"}),
        );
        let isolated = real.path().parent().unwrap().join(format!(
            "isolated-concurrent-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        prepare_claude_bridge_home(real.path(), &isolated).unwrap();

        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let missing = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let watcher = {
            let isolated = isolated.clone();
            let stop = stop.clone();
            let missing = missing.clone();
            std::thread::spawn(move || {
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    if !isolated.join(".credentials.json").exists() {
                        missing.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            })
        };

        let builders: Vec<_> = (0..4)
            .map(|_| {
                let real = real.path().to_path_buf();
                let isolated = isolated.clone();
                std::thread::spawn(move || {
                    for _ in 0..20 {
                        let _ = prepare_claude_bridge_home(&real, &isolated);
                    }
                })
            })
            .collect();
        for b in builders {
            b.join().unwrap();
        }
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        watcher.join().unwrap();

        let observed = missing.load(std::sync::atomic::Ordering::Relaxed);
        let _ = std::fs::remove_dir_all(&isolated);
        assert_eq!(
            observed, 0,
            "a concurrently-spawning claude saw CLAUDE_CONFIG_DIR without credentials {observed} times"
        );
    }

    #[test]
    fn missing_real_dir_is_a_noop_not_an_error() {
        let base = tempfile::tempdir().unwrap();
        let missing_real = base.path().join("does-not-exist");
        let isolated = base.path().join("isolated-missing");
        prepare_claude_bridge_home(&missing_real, &isolated).unwrap();
        assert!(!isolated.exists(), "nothing to mirror -> nothing built");
    }

    #[test]
    fn bridge_copy_fallback_allows_only_required_entries() {
        assert!(is_allowed_windows_copy(Path::new("projects")));
        assert!(is_allowed_windows_copy(Path::new("history.jsonl")));
        assert!(!is_allowed_windows_copy(Path::new(".credentials.json")));
        assert!(!is_allowed_windows_copy(Path::new("daemon")));
        assert!(!is_allowed_windows_copy(Path::new("control.key")));
    }

    #[test]
    fn malformed_settings_return_a_diagnostic_without_a_filtered_copy() {
        let real = tempfile::tempdir().unwrap();
        let settings = real.path().join("settings.json");
        std::fs::write(&settings, "{ definitely not json").unwrap();

        let error = filtered_settings_json(&settings).expect_err("malformed JSON is reported");
        assert!(!error.to_string().is_empty());
    }

    #[test]
    fn rebuilding_reflects_an_updated_fixture_not_stale_cache() {
        let real = tempfile::tempdir().unwrap();
        write_json(
            &real.path().join("settings.json"),
            &serde_json::json!({"theme": "dark"}),
        );
        let isolated = real.path().parent().unwrap().join("isolated-rebuild");
        prepare_claude_bridge_home(real.path(), &isolated).unwrap();
        let first: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(isolated.join("settings.json")).unwrap())
                .unwrap();
        assert_eq!(first["theme"], "dark");

        write_json(
            &real.path().join("settings.json"),
            &serde_json::json!({"theme": "light"}),
        );
        prepare_claude_bridge_home(real.path(), &isolated).unwrap();
        let second: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(isolated.join("settings.json")).unwrap())
                .unwrap();
        assert_eq!(second["theme"], "light", "rebuild is not stale-cached");

        let _ = std::fs::remove_dir_all(&isolated);
    }
}
