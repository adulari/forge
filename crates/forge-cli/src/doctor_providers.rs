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
