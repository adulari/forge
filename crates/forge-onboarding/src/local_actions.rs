//! Actions that run inside the engine: writes to the user config and the secret store, and the
//! local search container. Error strings never contain a secret.

use std::time::{Duration, Instant};

use forge_config::ConfigScope;
use forge_types::{CreditMode, PermissionMode};

use crate::action::{Secret, SecretSlot};
use crate::job::{ActionResult, JobHandle, Success};
use crate::proc::{run_capture, stream_child};

const SEARXNG_CONTAINER: &str = "forge-searxng";
const SEARXNG_PORT: u16 = 8888;

fn check_secret(secret: &Secret) -> Result<(), String> {
    let value = secret.expose();
    if value.is_empty() {
        return Err("Nothing was entered.".into());
    }
    if value.len() > 8192 || value.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(
            "That does not look like a single key or token: it has spaces or line breaks.".into(),
        );
    }
    Ok(())
}

pub(crate) fn store_key(provider: &str, key: &Secret, replace: bool) -> Result<(), String> {
    if !forge_config::known_key_providers().any(|p| p == provider) {
        return Err(format!("Forge does not know a provider called {provider}."));
    }
    check_secret(key)?;
    let stored = if replace {
        forge_config::store_api_key(provider, key.expose())
    } else {
        forge_config::add_api_key(provider, key.expose()).map(|_| ())
    };
    stored.map_err(|e| format!("Could not save the key: {e}"))?;
    forge_config::inject_provider_keys();
    Ok(())
}

pub(crate) fn store_secret(slot: SecretSlot, value: &Secret) -> Result<(), String> {
    check_secret(value)?;
    let stored = match slot {
        SecretSlot::ClaudeOauthToken | SecretSlot::Brave | SecretSlot::ArtificialAnalysis => {
            forge_config::store_api_key(slot.name(), value.expose())
        }
    };
    stored.map_err(|e| format!("Could not save it: {e}"))?;
    if slot == SecretSlot::Brave {
        forge_config::inject_search_keys();
    }
    Ok(())
}

fn credit_mode(key: &str) -> Option<CreditMode> {
    match key {
        "normal" => Some(CreditMode::Normal),
        "frugal" => Some(CreditMode::Frugal),
        "strict" => Some(CreditMode::Strict),
        _ => None,
    }
}

pub(crate) fn set_behavior(permission: &str, credit: &str) -> Result<Success, String> {
    let permission = PermissionMode::from_key(permission)
        .ok_or_else(|| format!("{permission} is not a permission mode."))?;
    let credit = credit_mode(credit).ok_or_else(|| format!("{credit} is not a credit mode."))?;
    forge_config::write_settings(permission, credit)
        .map_err(|e| format!("Could not save the settings: {e}"))?;
    Ok(Success::with(
        format!("{} · {}", permission.label(), credit.label()),
        ActionResult::Changed,
    ))
}

pub(crate) fn set_keybind_preset(preset: &str) -> Result<Success, String> {
    let preset = match preset {
        "claude-code" => forge_config::KeybindPreset::ClaudeCode,
        "forge" => forge_config::KeybindPreset::Forge,
        other => return Err(format!("{other} is not a key binding preset.")),
    };
    forge_config::write_keybind_preset(preset)
        .map_err(|e| format!("Could not save the key bindings: {e}"))?;
    Ok(Success::with("Key bindings saved.", ActionResult::Changed))
}

fn set_user_value(path: &str, value: &str) -> Result<(), String> {
    forge_config::set_config_value(ConfigScope::User, path, value)
        .map_err(|e| format!("Could not save {path}: {e}"))
}

pub(crate) fn set_notifications(enabled: bool) -> Result<Success, String> {
    set_user_value(
        "notifications.enabled",
        if enabled { "true" } else { "false" },
    )?;
    Ok(Success::with(
        "Notification setting saved.",
        ActionResult::Changed,
    ))
}

pub(crate) fn set_account_sync(category: &str, enabled: bool) -> Result<Success, String> {
    if !forge_config::AccountSyncConfig::CATEGORIES.contains(&category) {
        return Err(format!("{category} is not a sync category."));
    }
    set_user_value(
        &format!("anywhere.account_sync.{category}"),
        if enabled { "true" } else { "false" },
    )?;
    Ok(Success::with("Sync setting saved.", ActionResult::Changed))
}

pub(crate) fn set_theme(theme: &str) -> Result<Success, String> {
    if !matches!(theme, "auto" | "dark" | "light") {
        return Err(format!("{theme} is not a theme."));
    }
    set_user_value("tui.theme", theme)?;
    Ok(Success::with("Theme saved.", ActionResult::Changed))
}

pub(crate) fn set_local_model(tag: &str, autostart: bool) -> Result<Success, String> {
    if tag.is_empty() || tag.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("That is not a model tag.".into());
    }
    set_user_value("local.model", tag)?;
    set_user_value("local.autostart", if autostart { "true" } else { "false" })?;
    Ok(Success::with("Local model saved.", ActionResult::Changed))
}

fn searxng_settings(secret_key: &str) -> String {
    format!(
        "use_default_settings: true\nserver:\n  secret_key: \"{secret_key}\"\n  limiter: false\n  image_proxy: false\nsearch:\n  formats:\n    - html\n    - json\n"
    )
}

/// Start (or restart) the `forge-searxng` container Forge's web search already looks for on
/// `localhost:8888`, with the JSON format it needs switched on.
pub(crate) fn start_searxng(job: &JobHandle) -> Result<Success, String> {
    let docker_up = run_capture(
        "docker",
        &["info", "--format", "{{.ServerVersion}}"],
        Duration::from_secs(5),
    )
    .is_some_and(|(ok, _)| ok);
    if !docker_up {
        return Err("Docker is not running. Start Docker and try again.".into());
    }
    let dir = forge_config::data_dir()
        .ok_or("Forge has no data directory to keep the search settings in.")?
        .join("searxng");
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
    let settings = dir.join("settings.yml");
    if !settings.exists() {
        let secret = forge_config::random_state();
        std::fs::write(&settings, searxng_settings(&secret))
            .map_err(|e| format!("Could not write the search settings: {e}"))?;
    }

    let existing = run_capture(
        "docker",
        &[
            "ps",
            "-a",
            "-q",
            "--filter",
            &format!("name=^{SEARXNG_CONTAINER}$"),
        ],
        Duration::from_secs(5),
    )
    .map(|(_, out)| !out.trim().is_empty())
    .unwrap_or(false);
    let args: Vec<String> = if existing {
        vec!["start".into(), SEARXNG_CONTAINER.into()]
    } else {
        vec![
            "run".into(),
            "-d".into(),
            "--name".into(),
            SEARXNG_CONTAINER.into(),
            "--restart".into(),
            "unless-stopped".into(),
            "-p".into(),
            format!("127.0.0.1:{SEARXNG_PORT}:8080"),
            "-v".into(),
            format!("{}:/etc/searxng", dir.display()),
            "searxng/searxng".into(),
        ]
    };
    job.progress("Starting the search container", None);
    stream_child(job, "docker", &args)?;

    job.progress("Waiting for it to answer", None);
    let deadline = Instant::now() + Duration::from_secs(25);
    while Instant::now() < deadline {
        if std::net::TcpStream::connect_timeout(
            &([127, 0, 0, 1], SEARXNG_PORT).into(),
            Duration::from_millis(300),
        )
        .is_ok()
        {
            return Ok(Success::with(
                format!("Local search is running on localhost:{SEARXNG_PORT}."),
                ActionResult::Changed,
            ));
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Err("The container started but is not answering yet. Give it a minute, then check `docker logs forge-searxng`.".into())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::test_support::{env_lock, ScopedEnv};
    use std::ffi::OsStr;

    fn isolated() -> (
        std::sync::MutexGuard<'static, ()>,
        ScopedEnv,
        tempfile::TempDir,
    ) {
        let guard = env_lock();
        let home = tempfile::tempdir().unwrap();
        let config = home.path().join("config");
        let data = home.path().join("data");
        let env = ScopedEnv::set(&[
            ("HOME", Some(home.path().as_os_str())),
            ("XDG_CONFIG_HOME", Some(config.as_os_str())),
            ("XDG_DATA_HOME", Some(data.as_os_str())),
            ("FORGE_DB", None),
            ("GROQ_API_KEY", None),
            ("LANG", Some(OsStr::new("en_US.UTF-8"))),
        ]);
        (guard, env, home)
    }

    fn config_text() -> String {
        let path = forge_config::config_dir().unwrap().join("config.toml");
        std::fs::read_to_string(path).unwrap_or_default()
    }

    #[test]
    fn behavior_settings_are_written_to_the_user_config() {
        let (_g, _e, _home) = isolated();
        set_behavior("accept-edits", "frugal").unwrap();
        let text = config_text();
        assert!(text.contains("accept-edits"), "{text}");
        assert!(text.contains("frugal"), "{text}");
        assert!(set_behavior("yolo", "frugal").is_err());
        assert!(set_behavior("default", "free-for-all").is_err());
    }

    #[test]
    fn notifications_theme_and_local_model_land_in_the_config() {
        let (_g, _e, _home) = isolated();
        set_notifications(true).unwrap();
        set_theme("dark").unwrap();
        set_local_model("qwen2.5-coder:7b", true).unwrap();
        let text = config_text();
        assert!(
            text.contains("[notifications]") && text.contains("enabled = true"),
            "{text}"
        );
        assert!(text.contains("theme = \"dark\""), "{text}");
        assert!(text.contains("qwen2.5-coder:7b"), "{text}");
        assert!(set_theme("neon").is_err());
        assert!(set_local_model("two words", false).is_err());
    }

    #[test]
    fn the_keybind_preset_is_written_and_validated() {
        let (_g, _e, _home) = isolated();
        set_keybind_preset("claude-code").unwrap();
        assert!(config_text().contains("claude-code"));
        assert!(set_keybind_preset("emacs").is_err());
    }

    #[test]
    fn a_stored_key_reaches_the_secret_store_and_a_bad_one_is_refused_without_echoing_it() {
        let (_g, _e, _home) = isolated();
        store_key("groq", &Secret::new("gsk-test-0123456789"), true).unwrap();
        assert!(forge_config::has_api_key("groq"));
        forge_config::remove_api_key("groq").unwrap();

        let err = store_key("groq", &Secret::new("two words"), false).unwrap_err();
        assert!(!err.contains("two words"));
        assert!(store_key("not-a-provider", &Secret::new("abc"), false).is_err());
        assert!(store_key("groq", &Secret::new("   "), false).is_err());
    }

    #[test]
    fn secret_slots_store_under_their_own_names() {
        let (_g, _e, _home) = isolated();
        store_secret(
            SecretSlot::ClaudeOauthToken,
            &Secret::new("sk-ant-oat01-abcdef"),
        )
        .unwrap();
        assert!(crate::detect::claude_token_stored());
        forge_config::secret_store::delete(forge_config::CLAUDE_OAUTH_TOKEN_SLOT).unwrap();
        assert!(!crate::detect::claude_token_stored());

        store_secret(SecretSlot::Brave, &Secret::new("BSA-test-key")).unwrap();
        assert!(forge_config::has_search_key("brave"));
        forge_config::remove_api_key("brave").unwrap();
    }

    #[test]
    fn the_searxng_settings_switch_the_json_format_on() {
        let yaml = searxng_settings("s3cret");
        assert!(yaml.contains("- json"));
        assert!(yaml.contains("secret_key: \"s3cret\""));
        assert!(yaml.contains("limiter: false"));
    }
}
