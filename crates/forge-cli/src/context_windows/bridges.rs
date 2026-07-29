//! CLI bridge context-window derivation and authoritative overrides.

use std::collections::HashMap;

use forge_mesh::pricing;

// ── CLI bridge derivation ─────────────────────────────────────────────────────────────────────────

/// Persist narrowly scoped authoritative context windows after every discovery source. Most model
/// windows remain API-derived; this covers compatible endpoints that expose only model IDs.
pub(super) fn persist_authoritative_contexts(models: &[String], store: &forge_store::Store) {
    for model in models {
        if let Some(window) = pricing::authoritative_context_limit(model) {
            let _ = store.set_model_context(model, window);
        }
    }
}

/// Family word a bridge's bare tier aliases must share with a canonical candidate. claude-cli
/// aliases (`opus`, `fable`) and codex-cli's don't always spell their family, so without this
/// `opus` could match any model containing that word. agy-cli aliases are full model names
/// (`gemini-3.5-flash`, `claude-sonnet-4.6`) that already carry their family.
fn bridge_family_token(prefix: &str) -> Option<&'static str> {
    match prefix {
        "claude-cli" => Some("claude"),
        "codex-cli" => Some("gpt"),
        _ => None,
    }
}

/// The context window for one bridge alias, derived from fetched data: the alias's token set must
/// be a subset of a canonical model's tokens — the same vocabulary the benchmark mapper uses
/// (`fable` ⊆ `anthropic::claude-fable-5`) — and among all canonical matches the LARGEST window
/// wins, because a versionless alias (`opus`, `sonnet`) means the latest/best model of that tier,
/// exactly like its benchmark mapping. `None` when nothing fetched matches.
pub(super) fn bridge_window(
    prefix: &str,
    alias: &str,
    ctx_registry: &HashMap<String, u32>,
) -> Option<u32> {
    let want = forge_mesh::bench::tokens(alias);
    if want.is_empty() {
        return None;
    }
    let family = bridge_family_token(prefix);
    ctx_registry
        .iter()
        .filter(|(id, _)| {
            let namespace = id.split_once("::").map(|(ns, _)| ns).unwrap_or_default();
            let canonical_namespace = match prefix {
                "claude-cli" => namespace == "anthropic",
                "codex-cli" => namespace == "openai",
                "agy-cli" => match alias.split('-').next().unwrap_or_default() {
                    "claude" => namespace == "anthropic",
                    "gemini" => namespace == "gemini",
                    "gpt" => namespace == "openai",
                    _ => matches!(namespace, "anthropic" | "gemini" | "openai"),
                },
                _ => true,
            };
            let model = id.split_once("::").map(|(_, m)| m).unwrap_or(id);
            let cand = forge_mesh::bench::tokens(model);
            canonical_namespace
                && family.is_none_or(|f| cand.iter().any(|t| t == f))
                && want.iter().all(|t| cand.contains(t))
        })
        .map(|(_, &w)| w)
        .max()
}

pub(super) fn derive_cli_bridge_windows(
    models: &[String],
    ctx_registry: &HashMap<String, u32>,
    store: &forge_store::Store,
) {
    for id in models {
        let Some((prefix, alias)) = id.split_once("::") else {
            continue;
        };
        if alias.is_empty() || !matches!(prefix, "claude-cli" | "codex-cli" | "agy-cli") {
            continue;
        }
        // Dynamic first. The hardcoded per-bridge table (forge_mesh::pricing::context_limit) is
        // the LAST-RESORT fallback, for a bridge model absent from every fetched source (e.g. a
        // codex GPT release before OpenRouter lists it). The result is written to the store
        // either way so a stale row from an earlier run can't linger and win over the fallback
        // at read time (the store outranks context_limit in effective_context_window).
        let window =
            bridge_window(prefix, alias, ctx_registry).or_else(|| pricing::context_limit(id));
        if let Some(w) = window {
            let _ = store.set_model_context(id, w);
        }
    }
}
