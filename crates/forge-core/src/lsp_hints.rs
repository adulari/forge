//! Turning a language server's answer about a just-written file into a hint for the model.
//!
//! Split out of `tool_dispatch` because the interesting decision here is not dispatch, it is
//! EDITORIAL: what is worth spending the model's context on. A real diagnostic is worth it — the
//! model can fix the line this turn. An outage notice is not: the model cannot act on it, and it
//! arrives after every single write for as long as the server is down. One project emitted 386 of
//! them, each one context the turn paid to carry and then re-send on every subsequent step.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use forge_config::LspDiagnosticsMode;
use forge_lsp::types::Diagnostic;
use forge_lsp::LspRegistry;

/// The file's text just before a write, captured only when the delta needs it.
pub(crate) struct PreEdit {
    path: PathBuf,
    text: Option<String>,
}

impl crate::Session {
    /// A tool's path argument is relative to the workspace, which is not always the process cwd.
    fn lsp_abs_path(&self, path: &Path) -> PathBuf {
        let joined = self.workspace.root().join(path);
        std::path::absolute(&joined).unwrap_or(joined)
    }

    fn lsp_diagnostics_mode(&self) -> Option<LspDiagnosticsMode> {
        (self.config.lsp.enabled && self.lsp.is_some())
            .then_some(self.config.lsp.diagnostics)
            .filter(|mode| *mode != LspDiagnosticsMode::Off)
    }

    /// Call before a permitted write: the delta diffs the edit against this text.
    pub(crate) fn lsp_pre_edit(&self, path: &Path) -> Option<PreEdit> {
        (self.lsp_diagnostics_mode()? == LspDiagnosticsMode::Delta).then(|| {
            let path = self.lsp_abs_path(path);
            let text = std::fs::read_to_string(&path).ok();
            PreEdit { path, text }
        })
    }

    /// Queue the `[lsp diagnostics]` hint for a just-written file. Best-effort: no server, a
    /// clean file, or nothing new since the model last saw it all queue nothing.
    pub(crate) async fn queue_lsp_hint(&mut self, path: &Path, pre: Option<PreEdit>) {
        let Some(mode) = self.lsp_diagnostics_mode() else {
            return;
        };
        let Some(lsp) = self.lsp.clone() else {
            return;
        };
        let timeout = Duration::from_millis(self.config.lsp.timeout_ms);
        let hint = match (mode, pre) {
            (LspDiagnosticsMode::Delta, Some(pre)) => {
                let diagnostics = findings(&lsp, &pre.path, timeout).await;
                let Ok(after) = std::fs::read_to_string(&pre.path) else {
                    return;
                };
                let label = pre
                    .path
                    .strip_prefix(self.workspace.root())
                    .unwrap_or(&pre.path)
                    .display()
                    .to_string();
                self.lsp_baselines.observe(
                    pre.path,
                    &label,
                    pre.text.as_deref(),
                    after,
                    diagnostics,
                )
            }
            _ => diagnostics_hint(&lsp, &self.lsp_abs_path(path), path, timeout).await,
        };
        if let Some(hint) = hint {
            self.pending_hints.push(hint);
        }
    }
}

/// What the server reported, minus outage notices: the model cannot act on those, and they
/// arrive after every single write for as long as the server is down.
async fn findings(lsp: &Arc<LspRegistry>, abs: &Path, timeout: Duration) -> Vec<Diagnostic> {
    let mut diagnostics = lsp.diagnostics_for(abs, timeout).await;
    diagnostics.retain(|d| d.code.as_deref() != Some("forge-lsp-unavailable"));
    diagnostics
}

/// The full `[lsp diagnostics]` hint for `path` (`[lsp] diagnostics = "full"`), or `None` when
/// there is nothing worth saying.
///
/// `None` covers both "the file is clean" and "the server is down" — deliberately the same
/// outcome, because neither gives the model anything to do.
pub(crate) async fn diagnostics_hint(
    lsp: &Arc<LspRegistry>,
    abs: &Path,
    path: &Path,
    timeout: Duration,
) -> Option<String> {
    let lines: Vec<String> = findings(lsp, abs, timeout)
        .await
        .iter()
        .map(|d| d.format_line(&path.display().to_string()))
        .collect();
    (!lines.is_empty()).then(|| format!("[lsp diagnostics]\n{}", lines.join("\n")))
}

#[cfg(test)]
mod tests {
    use forge_lsp::types::{Diagnostic, DiagnosticSeverity};

    /// The filter is the whole point: an outage must not reach the model's context.
    #[test]
    fn an_outage_notice_is_not_a_finding() {
        let outage = Diagnostic {
            severity: DiagnosticSeverity::Information,
            message: "LSP diagnostics unavailable for x.rs: server died".into(),
            line: 0,
            character: 0,
            code: Some("forge-lsp-unavailable".to_string()),
        };
        let real = Diagnostic {
            severity: DiagnosticSeverity::Error,
            message: "cannot find value `x`".into(),
            line: 12,
            character: 4,
            code: Some("E0425".to_string()),
        };
        let keep = |d: &Diagnostic| d.code.as_deref() != Some("forge-lsp-unavailable");
        assert!(!keep(&outage), "an outage notice must be dropped");
        assert!(keep(&real), "a real diagnostic must survive");
    }
}
