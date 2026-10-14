//! `[lsp] diagnostics`: how much of a language server's post-edit report reaches the model.

use serde::{Deserialize, Serialize};

/// What Forge attaches after an edit when `[lsp] enabled = true`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LspDiagnosticsMode {
    /// Only what changed since the model last saw the file: new diagnostics, a resolved count,
    /// and on first touch only those near the edit. Errors the edit introduced are always shown.
    #[default]
    Delta,
    /// Every diagnostic for the file after every edit (the pre-delta behaviour).
    Full,
    /// Attach no diagnostics after edits; the language server is not asked.
    Off,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Config, LspConfig};

    #[test]
    fn defaults_to_delta_and_parses_each_value() {
        assert_eq!(Config::default().lsp.diagnostics, LspDiagnosticsMode::Delta);
        for (raw, want) in [
            ("delta", LspDiagnosticsMode::Delta),
            ("full", LspDiagnosticsMode::Full),
            ("off", LspDiagnosticsMode::Off),
        ] {
            let cfg: LspConfig = toml::from_str(&format!("diagnostics = \"{raw}\"")).unwrap();
            assert_eq!(cfg.diagnostics, want);
        }
        assert!(toml::from_str::<LspConfig>("diagnostics = \"some\"").is_err());
    }
}
