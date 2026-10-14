//! Test proof for the evidence ledger (`forge_tools::evidence`).
//!
//! A shell call that the completion policy classifies as a verification check (test, lint,
//! typecheck, build) and that exited 0 is recorded with its command, exit status and output tail.
//! The exit status is read from the tool result itself, so a check the model only claims to have
//! run never becomes evidence. Both the direct dispatcher and the CLI bridge's `mcp-serve` call
//! this, so the ledger agrees across surfaces.

use std::path::Path;

use crate::completion::{classify_tool, VerificationObservation};

impl crate::Session {
    /// Run a tool future inside this session's evidence scope, so captures land in its ledger.
    pub(crate) fn evidence_scope<F: std::future::Future>(
        &self,
        fut: F,
    ) -> impl std::future::Future<Output = F::Output> {
        forge_tools::scope_session(self.id.clone(), self.workspace.root().to_path_buf(), fut)
    }
}

/// Record a passing verification shell command as test proof. The exit status comes from
/// `result` (the model's copy, whose first line is the shell header); the tail comes from `full`,
/// the uncut output, when the model's copy was truncated.
pub fn record_verification_proof(
    workspace: &Path,
    session: &str,
    tool: &str,
    args: &serde_json::Value,
    result: &str,
    full: Option<&str>,
) {
    if tool != "shell" {
        return;
    }
    if !matches!(
        classify_tool(tool, &args.to_string()),
        VerificationObservation::Check(_)
    ) {
        return;
    }
    if shell_exit_code(result) != Some(0) {
        return;
    }
    let Some(command) = forge_types::shell_command_text(args) else {
        return;
    };
    let Some(ledger) = forge_tools::evidence::Ledger::new(workspace, session) else {
        return;
    };
    if let Err(error) = ledger.record_test_output(&command, 0, full.unwrap_or(result)) {
        tracing::debug!(%error, "could not record test evidence");
    }
}

impl crate::Session {
    pub(crate) fn record_test_proof(
        &self,
        tool: &str,
        args: &serde_json::Value,
        result: &str,
        full: Option<&str>,
    ) {
        record_verification_proof(self.workspace.root(), &self.id, tool, args, result, full);
    }
}

/// The bridge's `mcp-serve` child: the session comes from the env the parent set, the workspace
/// from the cwd it entered.
pub fn record_bridge_verification_proof(tool: &str, args: &serde_json::Value, output: &str) {
    let Ok(session) = std::env::var(crate::snapshot::ENV_SESSION) else {
        return;
    };
    let Ok(workspace) = std::env::current_dir() else {
        return;
    };
    record_verification_proof(&workspace, &session, tool, args, output, None);
}

fn shell_exit_code(result: &str) -> Option<i32> {
    result
        .lines()
        .next()?
        .strip_prefix("shell: exit ")?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(workspace: &Path) -> Vec<forge_tools::evidence::EvidenceItem> {
        forge_tools::evidence::Ledger::new(workspace, "s1")
            .unwrap()
            .items()
    }

    #[test]
    fn a_passing_test_command_is_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let args = serde_json::json!({"command": "cargo test -p demo"});
        record_verification_proof(
            dir.path(),
            "s1",
            "shell",
            &args,
            "shell: exit 0 in 812ms\ntest result: ok. 3 passed",
            None,
        );
        let recorded = items(dir.path());
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].command.as_deref(), Some("cargo test -p demo"));
        assert_eq!(recorded[0].exit_code, Some(0));
    }

    #[test]
    fn failures_non_checks_and_masked_status_are_not_proof() {
        let dir = tempfile::tempdir().unwrap();
        let cases = [
            ("shell", "cargo test", "shell: exit 101 in 3s\nFAILED"),
            ("shell", "ls -la", "shell: exit 0 in 1ms\n."),
            ("shell", "cargo test || true", "shell: exit 0 in 1ms\n"),
            ("read_file", "cargo test", "shell: exit 0 in 1ms\n"),
            ("shell", "cargo test", "error: permission denied by policy"),
        ];
        for (tool, command, output) in cases {
            let args = serde_json::json!({ "command": command });
            record_verification_proof(dir.path(), "s1", tool, &args, output, None);
        }
        assert!(items(dir.path()).is_empty());
    }
}
