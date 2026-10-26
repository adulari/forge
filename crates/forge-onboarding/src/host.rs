//! What the engine needs from whoever embeds it. The `forge` binary owns the service manager,
//! Ollama and Anywhere code, so it answers those questions; the engine asks through this trait
//! and never links the binary crate.

use std::path::PathBuf;

use crate::machine::{AnywhereFacts, LocalFacts, ServiceFacts};

pub trait Host: Send + Sync {
    /// The `forge` executable that action jobs run as a child (`forge service install`, ...).
    fn forge_exe(&self) -> Option<PathBuf> {
        std::env::current_exe().ok()
    }

    fn forge_version(&self) -> String {
        env!("CARGO_PKG_VERSION").to_string()
    }

    /// Local-model facts: Ollama state, installed models, the pick that fits this machine.
    fn local(&self) -> LocalFacts {
        LocalFacts::default()
    }

    /// The command that installs Ollama on this OS, if there is an unattended one.
    fn ollama_install_argv(&self) -> Option<Vec<String>> {
        None
    }

    /// A credential for `provider` (a provider id or a bridge prefix) was just stored or replaced.
    /// The host clears whatever it remembers about the old one (a health exclusion, a cached
    /// model list).
    fn credentials_changed(&self, _provider: &str) {}

    fn service(&self) -> ServiceFacts {
        ServiceFacts::default()
    }

    fn anywhere(&self) -> AnywhereFacts {
        AnywhereFacts::default()
    }
}

/// A host that knows nothing beyond the engine itself: used by tests and by callers that only
/// need the pure parts.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoHost;

impl Host for NoHost {}
