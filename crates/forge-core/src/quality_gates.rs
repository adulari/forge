//! Session post-turn review and autofix quality gates.
//!
//! This owner keeps diff construction, critic policy, and synthetic autofix feedback ordered.

use super::*;

impl Session {
    /// Build a unified diff of files written this turn (pre-turn blob vs current file), run the
    /// Assay critic crew over it, and surface findings whose severity >= `gate_severity`. In
    /// `warn` mode the findings are emitted as warnings and the turn continues. In `block` mode
    /// they are emitted and `CoreError::TurnBlocked` is returned so the turn is aborted.
    pub(crate) async fn auto_review_gate(
        &mut self,
        cfg: &forge_config::AssayConfig,
    ) -> Result<(), CoreError> {
        use similar::{ChangeTag, TextDiff};

        // Gather files touched this turn from the snapshot manifest.
        let turn_files = snapshot::changed_files_this_turn(
            &self.checkpoint_root,
            &self.id,
            self.current_turn_seq,
        );
        if turn_files.is_empty() {
            return Ok(());
        }

        // Build a concatenated unified diff: for each file, diff old (blob or empty) vs new.
        let mut combined = String::new();
        for tf in &turn_files {
            // Async path: read the snapshot blob + the post-edit file with `tokio::fs` so a slow or
            // networked filesystem can't stall the executor while the auto-review gate builds its diff.
            let old = match &tf.blob {
                Some(p) => tokio::fs::read_to_string(p).await.unwrap_or_default(),
                None => String::new(),
            };
            let new = tokio::fs::read_to_string(&tf.path)
                .await
                .unwrap_or_default();
            if old == new {
                continue;
            }
            combined.push_str(&format!("--- a/{}\n+++ b/{}\n", tf.path, tf.path));
            let td = TextDiff::from_lines(old.as_str(), new.as_str());
            for change in td.iter_all_changes() {
                let sym = match change.tag() {
                    ChangeTag::Delete => "-",
                    ChangeTag::Insert => "+",
                    ChangeTag::Equal => " ",
                };
                combined.push_str(&format!("{sym} {}", change.value()));
            }
            combined.push('\n');
        }

        if combined.len() < cfg.min_diff_bytes {
            return Ok(());
        }

        self.presenter.emit(PresenterEvent::Warning(format!(
            "auto-review: diff is {} bytes — running critic crew",
            combined.len(),
        )));

        let lenses = forge_types::FindingCategory::crew().to_vec();
        let pricing = std::sync::Arc::new(self.pricing.clone());
        let provider = std::sync::Arc::clone(&self.provider);
        let store = std::sync::Arc::clone(&self.store);
        let cooldown = std::time::Duration::from_secs(self.config.mesh.failover_cooldown_secs);

        // Build tier model chains from the catalog (ranked + health-filtered) when available,
        // falling back to the configured model list — same pattern as the CLI's /assay path.
        let benched = self.provider_readiness().health;
        let models = {
            let chain = |tier: forge_types::TaskTier| -> Vec<String> {
                // Catalog path: ranked candidates, drop currently-benched ones first.
                if let Some(cat) = &self.catalog {
                    let ranked: Vec<String> = cat
                        .ranked_for(tier, &self.pricing, 8)
                        .into_iter()
                        .filter(|m| !benched.is_benched(m))
                        .collect();
                    if !ranked.is_empty() {
                        return ranked;
                    }
                }
                // Config fallback: the configured candidates for this tier.
                self.config
                    .candidates_for(tier)
                    .into_iter()
                    .filter(|m| !benched.is_benched(m))
                    .collect()
            };
            assay::TierModels {
                trivial: chain(forge_types::TaskTier::Trivial),
                complex: chain(forge_types::TaskTier::Complex),
            }
        };

        // Cost pre-estimate: skip the gate (with a warning) when the estimated crew cost exceeds
        // the configured cap. This prevents the gate from running away cost on large diffs.
        // cap == 0.0 means unlimited — always run.
        if cfg.max_cost_usd > 0.0 {
            let est = assay::estimate_assay_cost(&combined, &lenses, &models, &self.pricing);
            if est.est_usd > cfg.max_cost_usd {
                self.presenter.emit(PresenterEvent::Warning(format!(
                    "assay gate skipped: estimated ${:.3} exceeds cap ${:.3}",
                    est.est_usd, cfg.max_cost_usd,
                )));
                return Ok(());
            }
        }

        let source: std::sync::Arc<str> = combined.into();
        let presenter = &mut self.presenter;
        let mut on_progress = |p: assay::AssayProgress| {
            presenter.emit(PresenterEvent::AssayProgress(assay::progress_line(&p)));
        };

        let report = assay::run_assay(
            forge_types::AssayScope::Diff,
            source,
            lenses,
            models,
            provider,
            pricing,
            store,
            cooldown,
            &mut on_progress,
        )
        .await;

        // Filter to findings at/above the configured gate severity.
        let gate_findings: Vec<&forge_types::Finding> = report
            .findings
            .iter()
            .filter(|f| severity_meets(f.severity, &cfg.gate_severity))
            .collect();

        if gate_findings.is_empty() {
            self.presenter.emit(PresenterEvent::Warning(
                "auto-review: no findings at/above gate severity — OK".to_string(),
            ));
            return Ok(());
        }

        // Surface all gate-triggering findings as warnings.
        for f in &gate_findings {
            self.presenter.emit(PresenterEvent::Warning(format!(
                "auto-review [{}] {}: {} — {} ({}:{})",
                f.severity.as_str(),
                f.category.as_str(),
                f.title,
                f.suggested_fix,
                f.file,
                f.line.map(|l| l.to_string()).unwrap_or_default(),
            )));
        }

        if cfg.gate_mode.trim().eq_ignore_ascii_case("block") {
            return Err(CoreError::TurnBlocked(format!(
                "{} finding(s) at/above '{}' severity",
                gate_findings.len(),
                cfg.gate_severity
            )));
        }

        Ok(())
    }

    /// Run the autofix stage: execute lint and/or test commands (if enabled and non-empty);
    /// return `Ok(true)` when every enabled command exits 0, `Ok(false)` when any fails (the
    /// combined output of failing commands is injected into the transcript as a synthetic user
    /// message so the model can fix it next iteration). Never returns `Err` from a non-zero
    /// command exit — only from infrastructure failures (transcript write, etc.).
    /// Detect lint / test commands from project structure (zero-config autofix).
    /// Checks the current working directory — the project root where `forge chat` launched.
    /// Returns `(lint_cmd, test_cmd)` when a known project type is found; `test_cmd` is `None`
    /// when the project type has no obvious cheap test command.
    pub(crate) fn fill_detected_autofix_commands(
        config: &mut forge_config::AutofixConfig,
        lint: String,
        test: Option<String>,
    ) -> Vec<String> {
        let mut detected = Vec::with_capacity(2);
        if config.auto_lint && config.lint_cmd.is_empty() && !lint.is_empty() {
            detected.push(lint.clone());
            config.lint_cmd = lint;
        }
        if config.auto_test && config.test_cmd.is_empty() {
            if let Some(test) = test {
                detected.push(test.clone());
                config.test_cmd = test;
            }
        }
        detected
    }

    pub(crate) fn detect_project_commands(
        root: &std::path::Path,
    ) -> Result<Option<(String, Option<String>)>, String> {
        if root.join("Cargo.toml").exists() {
            return Ok(Some((
                "cargo check --all-targets 2>&1".to_string(),
                Some("cargo test --workspace 2>&1".to_string()),
            )));
        }
        if root.join("package.json").exists() {
            let package_path = root.join("package.json");
            let text = std::fs::read_to_string(&package_path)
                .map_err(|error| format!("cannot read {}: {error}", package_path.display()))?;
            let package = serde_json::from_str::<serde_json::Value>(&text)
                .map_err(|error| format!("cannot parse {}: {error}", package_path.display()))?;
            let Some(scripts) = package.get("scripts").and_then(|value| value.as_object()) else {
                return Ok(None);
            };
            let lint = ["lint", "typecheck", "check"]
                .into_iter()
                .find(|name| scripts.contains_key(*name))
                .map_or_else(String::new, |name| format!("npm run {name} 2>&1"));
            let test = scripts
                .contains_key("test")
                .then(|| "npm test 2>&1".to_string());
            return Ok(Some((lint, test)));
        }
        if root.join("pyproject.toml").exists() || root.join("requirements.txt").exists() {
            return Ok(Some((
                "python -m pytest --tb=short -q 2>&1".to_string(),
                None,
            )));
        }
        if root.join("go.mod").exists() {
            return Ok(Some((
                "go build ./... 2>&1".to_string(),
                Some("go test ./... 2>&1".to_string()),
            )));
        }
        Ok(None)
    }

    pub(crate) async fn run_autofix_stage(
        &mut self,
        af: &forge_config::AutofixConfig,
    ) -> Result<bool, CoreError> {
        // Use the same 120-second timeout as the shell tool's default; lint/test commands that
        // need more can be wrapped in a script.
        const AUTOFIX_TIMEOUT_SECS: u64 = 120;
        let mut failures = Vec::new();

        if af.auto_lint && !af.lint_cmd.is_empty() {
            let out = forge_tools::run_shell_command(
                &af.lint_cmd,
                &self.workspace.display(),
                AUTOFIX_TIMEOUT_SECS,
            )
            .await;
            if shell_command_failed(&out) {
                failures.push(format!("[lint: {}]\n{}", af.lint_cmd, out));
            }
        }
        if af.auto_test && !af.test_cmd.is_empty() {
            let out = forge_tools::run_shell_command(
                &af.test_cmd,
                &self.workspace.display(),
                AUTOFIX_TIMEOUT_SECS,
            )
            .await;
            if shell_command_failed(&out) {
                failures.push(format!("[test: {}]\n{}", af.test_cmd, out));
            }
        }

        if failures.is_empty() {
            return Ok(true);
        }

        // Inject the failures as a synthetic user message so the model fixes them on the next
        // iteration of the outer autofix loop.
        let body = format!(
            "Auto-fix: the following checks failed, fix them:\n\n{}",
            failures.join("\n\n")
        );
        let seq = self.next_seq();
        self.store
            .add_message(&self.id, seq, Role::User, &body, None)?;
        self.transcript.push(Message::user(&body));

        Ok(false)
    }
}
