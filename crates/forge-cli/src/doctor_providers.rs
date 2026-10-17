//! `[[providers.custom]]` rows for `forge doctor`, kept out of `doctor.rs` to keep it reviewable.

use crate::doctor::{check, Check, Status};

/// One row per user-defined endpoint; warns when its `api_key_env` variable is unset. Returns
/// whether at least one endpoint is usable.
pub(crate) fn custom_provider_checks(out: &mut Vec<Check>) -> bool {
    let Ok(cfg) = forge_config::load() else {
        return false;
    };
    let mut usable = false;
    for provider in &cfg.providers.custom {
        let name = format!("{} provider", provider.namespace);
        match provider.api_key_env.as_deref().filter(|v| !v.is_empty()) {
            Some(var) if std::env::var(var).map_or(true, |v| v.is_empty()) => {
                out.push(check(
                    Status::Warn,
                    &name,
                    format!("env var {var} is not set"),
                    Some("export it, or remove `api_key_env` for a keyless server"),
                ));
            }
            _ => {
                usable = true;
                out.push(check(Status::Ok, &name, &provider.base_url, None));
            }
        }
    }
    usable
}

/// The secret-store rows: which store this binary reads, then whether `secrets.enc` actually
/// decrypts (the backend line only says which store is selected, not that it is readable).
pub(crate) fn secret_store_checks(out: &mut Vec<Check>) {
    use forge_config::secret_health::FileStoreHealth;
    // A `forge` bin built by `cargo test` carries the in-memory `test-secrets` store, so "no keys"
    // from it says nothing about the user's setup.
    let backend = forge_config::secret_store::backend();
    out.push(check(
        if backend.is_blind() {
            Status::Warn
        } else {
            Status::Ok
        },
        "secret store",
        backend.describe(),
        backend.is_blind().then_some(
            "rebuild with `cargo build --bin forge` — this binary cannot see any stored key",
        ),
    ));
    if backend.is_blind() {
        return;
    }
    match forge_config::secret_health::file_store_health() {
        FileStoreHealth::Absent => {}
        FileStoreHealth::Healthy { entries } => out.push(check(
            Status::Ok,
            "secrets file",
            format!("{entries} stored secret(s) decrypt cleanly"),
            None,
        )),
        FileStoreHealth::Corrupt(detail) => out.push(check(
            Status::Fail,
            "secrets file",
            format!("corrupt: {detail}"),
            Some("stored keys in the encrypted file cannot be read; re-add them with `forge auth <provider>`"),
        )),
    }
}
