//! Zero-data-retention routing (`[privacy] zero_retention`, docs/features/zero-retention.md).
//!
//! When the mode is on, a model may only be routed to if its provider is known not to retain
//! prompts. "Known" is deliberately narrow: an entry in [`RETENTION_TABLE`] that is backed by how
//! Forge itself talks to the provider, or the user vouching for a provider in
//! `[privacy.retains]`. Every other provider is treated as retaining. Guessing a hosted provider's
//! retention policy would turn a privacy control into a false promise.

use forge_config::Config;

use crate::RoutingDecision;

/// One provider whose retention behaviour Forge can state from its own code.
#[derive(Debug, Clone, Copy)]
pub struct RetentionEntry {
    pub provider: &'static str,
    pub retains: bool,
    pub evidence: &'static str,
}

/// The built-in table. Hosted APIs are absent on purpose: their retention terms depend on the
/// account, plan and region, none of which Forge can see, so they fall under "unknown = retaining"
/// until the user vouches for them.
pub const RETENTION_TABLE: &[RetentionEntry] = &[RetentionEntry {
    provider: "ollama",
    retains: false,
    evidence: "local Ollama server: Forge calls OLLAMA_HOST (default localhost:11434), so prompts \
               stay on this machine; treated as retaining when OLLAMA_HOST points elsewhere or \
               the model is an Ollama cloud model (`:cloud` / `-cloud` tag)",
}];

/// A candidate the mode removed, with the reason shown by `forge mesh explain`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivacyDrop {
    pub model: String,
    pub reason: String,
}

/// Why `model` must not receive a request under the active privacy config, or `None` when it may.
pub fn retention_block(config: &Config, model: &str) -> Option<String> {
    retention_block_with(config, model, std::env::var("OLLAMA_HOST").ok().as_deref())
}

fn retention_block_with(config: &Config, model: &str, ollama_host: Option<&str>) -> Option<String> {
    if !config.privacy.zero_retention {
        return None;
    }
    let provider = forge_config::provider_of(model);
    if let Some(&retains) = config.privacy.retains.get(provider) {
        return retains.then(|| format!("{provider} is marked retaining in [privacy.retains]"));
    }
    let Some(entry) = RETENTION_TABLE.iter().find(|e| e.provider == provider) else {
        return Some(format!(
            "no zero-retention evidence for {provider}; unknown providers count as retaining"
        ));
    };
    if entry.retains {
        return Some(format!("{provider} retains prompts ({})", entry.evidence));
    }
    if provider == "ollama" {
        if !host_is_loopback(ollama_host.unwrap_or("localhost")) {
            return Some("OLLAMA_HOST points off this machine".to_string());
        }
        if is_ollama_cloud_model(model) {
            return Some(
                "Ollama cloud model: the local server forwards it to ollama.com".to_string(),
            );
        }
    }
    None
}

/// Ollama's hosted models (`gpt-oss:120b-cloud`, `kimi-k2:1t-cloud`, `foo:cloud`) are called
/// through the local server but run on ollama.com, so the prompt leaves this machine.
fn is_ollama_cloud_model(model: &str) -> bool {
    let name = model.split_once("::").map_or(model, |(_, name)| name);
    let tag = name.rsplit_once(':').map_or("", |(_, tag)| tag);
    tag == "cloud" || tag.ends_with("-cloud")
}

fn host_is_loopback(host: &str) -> bool {
    let rest = host.split_once("://").map_or(host, |(_, rest)| rest);
    let authority = rest.split('/').next().unwrap_or(rest);
    let name = if let Some(bracketed) = authority.strip_prefix('[') {
        bracketed.split(']').next().unwrap_or(bracketed)
    } else {
        authority
            .rsplit_once(':')
            .map_or(authority, |(name, _)| name)
    };
    matches!(name, "localhost" | "127.0.0.1" | "::1" | "0.0.0.0" | "")
}

/// Whether `model` may receive a request (always true with the mode off).
pub fn allows(config: &Config, model: &str) -> bool {
    retention_block(config, model).is_none()
}

/// The candidates among `models` the mode drops, in order, each once.
pub fn drops<'a>(config: &Config, models: impl IntoIterator<Item = &'a str>) -> Vec<PrivacyDrop> {
    let mut out: Vec<PrivacyDrop> = Vec::new();
    for model in models {
        if out.iter().any(|d| d.model == model) {
            continue;
        }
        if let Some(reason) = retention_block(config, model) {
            out.push(PrivacyDrop {
                model: model.to_string(),
                reason,
            });
        }
    }
    out
}

/// Last line of routing: no decision may name a retaining model. A routed (unpinned) pick moves to
/// its first allowed fallback; a pin, or a decision with nothing left, becomes unroutable with the
/// reason in its rationale, so the caller fails honestly instead of sending the prompt anyway.
pub fn enforce_on_decision(config: &Config, decision: &mut RoutingDecision) {
    if !config.privacy.zero_retention {
        return;
    }
    decision.fallbacks.retain(|m| allows(config, m));
    let Some(reason) = retention_block(config, &decision.model) else {
        return;
    };
    if !decision.pinned && !decision.fallbacks.is_empty() {
        let refused = std::mem::replace(&mut decision.model, decision.fallbacks.remove(0));
        decision.rationale.push_str(&format!(
            " — zero-retention mode skipped {refused} ({reason})"
        ));
        return;
    }
    decision.rationale.push_str(&format!(
        " — zero-retention mode refuses {} ({reason}); no zero-retention model is usable \
         (vouch for a provider with [privacy.retains] <provider> = false)",
        decision.model
    ));
    decision.fallbacks.clear();
    decision.unroutable = true;
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_types::TaskTier;

    fn zdr() -> Config {
        let mut config = Config::default();
        config.privacy.zero_retention = true;
        config
    }

    fn decision(model: &str, fallbacks: &[&str], pinned: bool) -> RoutingDecision {
        RoutingDecision {
            tier: TaskTier::Standard,
            model: model.into(),
            rationale: "why".into(),
            fallbacks: fallbacks.iter().map(|m| m.to_string()).collect(),
            pinned,
            unroutable: false,
            effort: None,
        }
    }

    #[test]
    fn mode_off_allows_everything() {
        let config = Config::default();
        assert!(allows(&config, "openrouter::x/y:free"));
        assert!(allows(&config, "groq::llama"));
    }

    #[test]
    fn unknown_providers_count_as_retaining_and_local_ollama_is_allowed() {
        let config = zdr();
        let reason = retention_block_with(&config, "groq::llama", None).unwrap();
        assert!(
            reason.contains("unknown providers count as retaining"),
            "{reason}"
        );
        assert!(retention_block_with(&config, "openrouter::a/b:free", None).is_some());
        assert!(retention_block_with(&config, "claude-cli::opus", None).is_some());
        assert_eq!(
            retention_block_with(&config, "ollama::llama3.2", None),
            None
        );
        assert_eq!(
            retention_block_with(&config, "ollama::llama3.2", Some("http://127.0.0.1:11434")),
            None
        );
        assert!(
            retention_block_with(&config, "ollama::llama3.2", Some("https://gpu.example.com"))
                .is_some()
        );
        for cloud in ["ollama::gpt-oss:120b-cloud", "ollama::glm-4.6:cloud"] {
            let reason = retention_block_with(&config, cloud, None).unwrap();
            assert!(reason.contains("ollama.com"), "{reason}");
        }
        assert_eq!(
            retention_block_with(&config, "ollama::qwen3-coder:30b", None),
            None
        );
    }

    #[test]
    fn user_overrides_win_over_the_table_and_the_unknown_default() {
        let mut config = zdr();
        config.privacy.retains.insert("openrouter".into(), false);
        config.privacy.retains.insert("ollama".into(), true);
        assert_eq!(retention_block_with(&config, "openrouter::a/b", None), None);
        let reason = retention_block_with(&config, "ollama::llama3.2", None).unwrap();
        assert!(reason.contains("[privacy.retains]"), "{reason}");
    }

    #[test]
    fn loopback_hosts_parse() {
        for host in [
            "localhost",
            "127.0.0.1:11434",
            "http://localhost:11434/",
            "[::1]:11434",
        ] {
            assert!(host_is_loopback(host), "{host}");
        }
        for host in ["10.0.0.5:11434", "https://ollama.com", "gpu.lan"] {
            assert!(!host_is_loopback(host), "{host}");
        }
    }

    #[test]
    fn routed_pick_moves_to_an_allowed_fallback() {
        let mut d = decision("groq::llama", &["openai::gpt", "ollama::llama3.2"], false);
        enforce_on_decision(&zdr(), &mut d);
        assert_eq!(d.model, "ollama::llama3.2");
        assert!(d.fallbacks.is_empty());
        assert!(!d.unroutable);
        assert!(d
            .rationale
            .contains("zero-retention mode skipped groq::llama"));
    }

    #[test]
    fn a_retaining_pin_or_an_empty_chain_fails_honestly() {
        let mut pinned = decision("openai::gpt", &["ollama::llama3.2"], true);
        enforce_on_decision(&zdr(), &mut pinned);
        assert!(pinned.unroutable);
        assert!(pinned.fallbacks.is_empty());
        assert!(pinned
            .rationale
            .contains("zero-retention mode refuses openai::gpt"));

        let mut empty = decision("groq::llama", &["openai::gpt"], false);
        enforce_on_decision(&zdr(), &mut empty);
        assert!(empty.unroutable);
    }

    #[test]
    fn drops_lists_each_refused_model_once() {
        let d = drops(&zdr(), ["groq::a", "ollama::b", "groq::a", "openai::c"]);
        let models: Vec<&str> = d.iter().map(|d| d.model.as_str()).collect();
        assert_eq!(models, ["groq::a", "openai::c"]);
    }

    fn router(config: Config, models: &[&str]) -> crate::HeuristicRouter {
        crate::HeuristicRouter::new(config)
            .with_availability(|_| true)
            .with_catalog(crate::ModelCatalog::new(
                models.iter().map(|m| m.to_string()).collect(),
            ))
    }

    fn route(r: &crate::HeuristicRouter) -> RoutingDecision {
        r.decide(
            TaskTier::Trivial,
            "test".into(),
            crate::BudgetState::default(),
            &forge_types::ModelHealth::default(),
            crate::RouteHints::from_prompt("rename a variable"),
            &forge_types::SubscriptionQuota::default(),
            None,
            false,
        )
    }

    #[test]
    fn the_router_never_picks_or_falls_back_to_a_retaining_provider() {
        if std::env::var_os("OLLAMA_HOST").is_some() {
            return;
        }
        let models = ["groq::llama-3.1-8b-instant", "ollama::llama3.2"];
        let open = route(&router(Config::default(), &models));
        assert!(
            open.model.starts_with("groq::")
                || open.fallbacks.iter().any(|m| m.starts_with("groq::"))
        );

        let d = route(&router(zdr(), &models));
        assert_eq!(d.model, "ollama::llama3.2");
        assert!(
            d.fallbacks.iter().all(|m| m.starts_with("ollama::")),
            "{:?}",
            d.fallbacks
        );
        assert!(!d.unroutable);
    }

    #[test]
    fn the_router_fails_honestly_when_every_candidate_retains() {
        let mut config = zdr();
        config.privacy.retains.insert("ollama".into(), true);
        let d = route(&router(
            config,
            &["groq::llama-3.1-8b-instant", "ollama::llama3.2"],
        ));
        assert!(d.unroutable, "{}", d.rationale);
        assert!(d.fallbacks.is_empty());
        assert!(d.rationale.contains("zero-retention"), "{}", d.rationale);
    }

    #[test]
    fn a_retaining_pin_is_refused_not_dispatched() {
        let r = router(zdr(), &["groq::llama-3.1-8b-instant"])
            .with_pin(Some("groq::llama-3.1-8b-instant".into()));
        let d = route(&r);
        assert!(d.unroutable);
        assert!(d
            .rationale
            .contains("zero-retention mode refuses groq::llama-3.1-8b-instant"));
    }

    #[test]
    fn explanation_names_every_dropped_candidate() {
        let r = router(zdr(), &["groq::llama-3.1-8b-instant", "openai::gpt-5.4"]);
        let e = r.explain(
            "rename a variable",
            crate::BudgetState::default(),
            &forge_types::ModelHealth::default(),
            &forge_types::SubscriptionQuota::default(),
            None,
            &forge_types::ProjectContext::default(),
        );
        let dropped: Vec<&str> = e.privacy_drops.iter().map(|d| d.model.as_str()).collect();
        assert!(
            dropped.contains(&"groq::llama-3.1-8b-instant"),
            "{dropped:?}"
        );
        assert!(dropped.contains(&"openai::gpt-5.4"), "{dropped:?}");
        assert!(e.candidates.iter().all(|c| !c.usable));

        let off = router(Config::default(), &["groq::llama-3.1-8b-instant"]).explain(
            "rename a variable",
            crate::BudgetState::default(),
            &forge_types::ModelHealth::default(),
            &forge_types::SubscriptionQuota::default(),
            None,
            &forge_types::ProjectContext::default(),
        );
        assert!(off.privacy_drops.is_empty());
    }
}
