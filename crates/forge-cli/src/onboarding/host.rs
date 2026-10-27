//! What the onboarding engine asks the `forge` binary: local models, the service manager,
//! Anywhere state, and what to forget when a credential changes.

use forge_onboarding::machine::{AnywhereFacts, LocalFacts, LocalRecommendation, ServiceFacts};
use forge_onboarding::Host;

pub(crate) struct CliHost;

/// The local model to suggest: the strongest catalog entry that fits the machine, capped at 32B
/// so the first download stays reasonable on a big workstation.
fn suggested_model(specs: &crate::local::SystemSpecs) -> Option<LocalRecommendation> {
    crate::local::recommend(specs)
        .into_iter()
        .find(|m| m.params_b <= 32.0)
        .map(|m| LocalRecommendation {
            tag: m.ollama_tag.to_string(),
            label: m.label.to_string(),
            memory_gb: m.min_memory_gb,
        })
}

impl Host for CliHost {
    fn forge_version(&self) -> String {
        env!("CARGO_PKG_VERSION").to_string()
    }

    fn local(&self) -> LocalFacts {
        let specs = crate::local::detect_specs();
        let installed = crate::local::ollama_installed();
        LocalFacts {
            ollama_installed: installed,
            ollama_serving: installed && crate::local::ollama_serving(),
            models: if installed {
                crate::local::ollama_installed_models()
            } else {
                Vec::new()
            },
            memory_gb: specs.model_memory_gb(),
            recommended: suggested_model(&specs),
        }
    }

    fn ollama_install_argv(&self) -> Option<Vec<String>> {
        let specs = crate::local::detect_specs();
        let (program, args) = crate::local::ollama_install_command(&specs)?;
        Some(std::iter::once(program.to_string()).chain(args).collect())
    }

    fn service(&self) -> ServiceFacts {
        let supported = cfg!(any(
            target_os = "linux",
            target_os = "macos",
            target_os = "windows"
        ));
        match crate::cli::commands::service::query_service_status() {
            Ok(status) => ServiceFacts {
                supported,
                installed: status.installed,
                running: status.running,
            },
            Err(_) => ServiceFacts {
                supported,
                ..ServiceFacts::default()
            },
        }
    }

    fn anywhere(&self) -> AnywhereFacts {
        AnywhereFacts {
            signed_in: crate::anywhere::signed_in(),
        }
    }

    fn credentials_changed(&self, provider: &str) {
        if let Ok(store) = crate::open_store() {
            crate::cli::commands::local::clear_auth_exclusion_in(&store, provider);
        }
        crate::cli::commands::models::invalidate_catalog_cache();
    }
}

#[cfg(test)]
pub(super) fn suggested_model_for_test(
    specs: &crate::local::SystemSpecs,
) -> Option<LocalRecommendation> {
    suggested_model(specs)
}
