//! A provider added to config.toml mid-process becomes known without a restart.
//!
//! Own integration binary: the registries are process-global and the test redirects
//! `XDG_CONFIG_HOME`, which would race with unrelated unit tests sharing a process.

use forge_config::{
    add_custom_provider, custom_provider, known_key_providers, provider_set_fingerprint,
    reload_provider_registries, remove_custom_provider, CustomProviderConfig,
};

static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn gateway() -> CustomProviderConfig {
    CustomProviderConfig {
        namespace: "modelgateway".into(),
        base_url: "http://127.0.0.1:9/v1".into(),
        api_key_env: Some("MODELGATEWAY_API_KEY_TEST".into()),
        free: false,
        models: Vec::new(),
        label: None,
        pricing_stat: None,
    }
}

#[test]
fn adding_and_removing_a_custom_provider_is_picked_up_by_a_reload() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_CONFIG_HOME", dir.path());
    std::env::set_var("HOME", dir.path());

    // Initialise the registry before the provider exists (the long-running process).
    assert!(custom_provider("modelgateway").is_none());
    assert!(!known_key_providers().any(|p| p == "modelgateway"));
    let before = provider_set_fingerprint();
    assert!(!reload_provider_registries(), "nothing changed yet");

    add_custom_provider(&gateway()).unwrap();
    assert!(
        custom_provider("modelgateway").is_none(),
        "no reload yet, so the process still holds its startup view"
    );
    assert!(reload_provider_registries(), "the new block is a change");
    let cp = custom_provider("modelgateway").expect("registered after reload");
    assert_eq!(cp.endpoint, "http://127.0.0.1:9/v1/");
    assert!(known_key_providers().any(|p| p == "modelgateway"));
    assert_ne!(provider_set_fingerprint(), before);
    assert!(!reload_provider_registries(), "second reload is a no-op");

    assert!(remove_custom_provider("modelgateway").unwrap());
    assert!(reload_provider_registries());
    assert!(custom_provider("modelgateway").is_none());
    assert_eq!(provider_set_fingerprint(), before);
}

#[test]
fn a_stored_key_changes_the_provider_set_fingerprint() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    std::env::remove_var("GROQ_API_KEY");
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_CONFIG_HOME", dir.path());
    std::env::set_var("HOME", dir.path());
    let before = provider_set_fingerprint();
    forge_config::secret_store::set("groq", "gsk_test").unwrap();
    assert_ne!(provider_set_fingerprint(), before);
    forge_config::secret_store::delete("groq").unwrap();
    assert_eq!(provider_set_fingerprint(), before);
}
