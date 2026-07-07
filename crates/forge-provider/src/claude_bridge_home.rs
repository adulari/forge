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

/// Top-level JSON keys stripped from a filtered settings file. `hooks` is the primary target;
/// `enabledPlugins`/`extraKnownMarketplaces` are defense-in-depth against a plugin registering its
/// own hooks — plugins are a secondary concern here, hooks is the one that must never fire.
const STRIPPED_KEYS: &[&str] = &["hooks", "enabledPlugins", "extraKnownMarketplaces"];

/// The REAL claude config dir to mirror: `$CLAUDE_CONFIG_DIR` if the user already has one set
/// (respect it rather than silently ignoring it), else `<home>/.claude`.
pub fn real_claude_config_dir() -> Option<std::path::PathBuf> {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR") {
        return Some(std::path::PathBuf::from(dir));
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(std::path::PathBuf::from(home).join(".claude"))
}

/// Build (or refresh) `isolated_dir` as a mirror of `real_home` (the user's real claude config
/// dir) via symlinks for everything EXCEPT the hook-bearing settings files, which get a filtered
/// copy instead (hooks stripped). Pointing a bridged `claude` subprocess at `isolated_dir` via
/// `CLAUDE_CONFIG_DIR` keeps auth/session/resume continuity fully intact while guaranteeing the
/// user's own hooks never fire during a Forge-driven turn.
///
/// Builds into a temp sibling directory first, then atomically renames it over `isolated_dir` — a
/// crash mid-build (or a concurrent bridge spawn) never leaves a half-populated or half-stale dir
/// live. Idempotent and cheap enough to call before every bridge turn (rebuilding is fine; the real
/// config dir is small and this only runs once per spawn, never a hot loop).
///
/// `real_home` not existing (nothing to mirror) is a no-op, not an error — claude then falls back
/// to its own default/unauthenticated behavior, which is an existing-behavior edge case, not a
/// regression introduced by this isolation.
pub fn prepare_claude_bridge_home(real_home: &Path, isolated_dir: &Path) -> anyhow::Result<()> {
    if !real_home.is_dir() {
        return Ok(());
    }
    let parent = isolated_dir
        .parent()
        .ok_or_else(|| anyhow::anyhow!("isolated claude bridge home has no parent directory"))?;
    std::fs::create_dir_all(parent)?;

    // Build fresh into a private temp dir beside the target, then atomically rename over it.
    let tmp_dir = parent.join(format!(
        ".{}.tmp-{}",
        isolated_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("claude-bridge-home"),
        std::process::id()
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
            // Missing/malformed settings file: skip silently rather than failing the whole
            // operation — a bridge run should degrade to "no settings", not error out.
            if let Some(filtered) = filtered_settings_json(&real_path) {
                std::fs::write(&dest_path, filtered)?;
            }
            continue;
        }

        symlink_through(&real_path, &dest_path)?;
    }

    if isolated_dir.exists() {
        std::fs::remove_dir_all(isolated_dir)?;
    }
    std::fs::rename(&tmp_dir, isolated_dir)?;
    Ok(())
}

/// Parse `path` as JSON and strip [`STRIPPED_KEYS`]; `None` if the file is missing or not valid
/// JSON (caller skips it silently rather than failing the whole mirror build).
fn filtered_settings_json(path: &Path) -> Option<Vec<u8>> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut value: serde_json::Value = serde_json::from_str(&text).ok()?;
    if let Some(obj) = value.as_object_mut() {
        for key in STRIPPED_KEYS {
            obj.remove(*key);
        }
    }
    serde_json::to_vec_pretty(&value).ok()
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
        tracing::warn!(
            "claude bridge home: failed to mirror {} on Windows (needs Developer Mode or admin): {e}",
            real_path.display()
        );
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

    #[test]
    fn missing_real_dir_is_a_noop_not_an_error() {
        let base = tempfile::tempdir().unwrap();
        let missing_real = base.path().join("does-not-exist");
        let isolated = base.path().join("isolated-missing");
        prepare_claude_bridge_home(&missing_real, &isolated).unwrap();
        assert!(!isolated.exists(), "nothing to mirror -> nothing built");
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
