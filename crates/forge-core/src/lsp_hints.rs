//! Turning a language server's answer about a just-written file into a hint for the model.
//!
//! Split out of `tool_dispatch` because the interesting decision here is not dispatch, it is
//! EDITORIAL: what is worth spending the model's context on. A real diagnostic is worth it — the
//! model can fix the line this turn. An outage notice is not: the model cannot act on it, and it
//! arrives after every single write for as long as the server is down. One project emitted 386 of
//! them, each one context the turn paid to carry and then re-send on every subsequent step.

use std::sync::Arc;
use std::time::Duration;

use forge_lsp::LspRegistry;

/// The `[lsp diagnostics]` hint for `path`, or `None` when there is nothing worth saying.
///
/// `None` covers both "the file is clean" and "the server is down" — deliberately the same
/// outcome, because neither gives the model anything to do.
pub(crate) async fn diagnostics_hint(
    lsp: &Arc<LspRegistry>,
    path: &std::path::Path,
    timeout: Duration,
) -> Option<String> {
    let abs = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let diagnostics = lsp.diagnostics_for(&abs, timeout).await;
    let lines: Vec<String> = diagnostics
        .iter()
        .filter(|d| d.code.as_deref() != Some("forge-lsp-unavailable"))
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
