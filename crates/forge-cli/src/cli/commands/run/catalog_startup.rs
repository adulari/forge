//! What starting a session owes the model catalog.
//!
//! The catalog (every provider's model list, prices, benchmarks) is what the mesh ranks when it
//! chooses a model. A session pinned to one fully-qualified model never chooses, so building the
//! catalog for it only costs: the first-ever discovery asks every installed agent CLI what it can
//! run (`agy models` alone started six MCP servers and held 1 GB for 1.8 s) and the first token of
//! a fresh install waited for it, 4.1 s against 1.4 s warm on `inferhub::cb/deepseek-v4.1-flash`.

use super::session::pin_members;

/// What startup does about the catalog for one session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct CatalogNeed {
    /// The session's routing reads the catalog, so a machine's first run waits (bounded) for it
    /// instead of routing from the built-in seeds.
    pub routes_by_catalog: bool,
    /// A rediscovery beside the session is worth starting. It only ever helps the NEXT startup, and
    /// a one-shot that is not going to route through the catalog exits before it finishes.
    pub refresh: bool,
}

/// `pin` is the normalized `--model` / stored pin. `attended` is whether a person is at the
/// session (the TUI) rather than a script (`forge run -p`, a bench task).
pub(super) fn catalog_need(pin: Option<&str>, attended: bool, pin_failover: bool) -> CatalogNeed {
    let Some(pin) = pin else {
        return CatalogNeed {
            routes_by_catalog: true,
            refresh: true,
        };
    };
    // One pinned model that names its provider is dispatched as it is. A set ranks its members
    // through the catalog, `pin_failover` falls back across it, and a bare alias is resolved by it.
    let members = pin_members(pin);
    let strict = !pin_failover
        && members.len() == 1
        && members
            .iter()
            .all(|member| !forge_config::provider_of(member).is_empty());
    CatalogNeed {
        // An attended session outlives the detached refresh, so it never holds its first frame for
        // one (a pinned TUI) and the catalog it reads is the one the refresh lands behind it.
        routes_by_catalog: !attended && !strict,
        refresh: attended || !strict,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GATEWAY: &str = "inferhub::cb/deepseek-v4.1-flash";

    #[test]
    fn a_pinned_one_shot_builds_no_catalog() {
        let need = catalog_need(Some(GATEWAY), false, false);
        assert!(
            !need.routes_by_catalog,
            "must not wait for first-run discovery"
        );
        assert!(!need.refresh, "must not start probes it will not outlive");
    }

    #[test]
    fn a_bridge_pin_is_as_strict_as_a_gateway_pin() {
        for pin in [
            "claude-cli::haiku",
            "codex-cli::gpt-6-luna",
            "agy-cli::gemini-3.5-flash",
        ] {
            let need = catalog_need(Some(pin), false, false);
            assert_eq!(
                need,
                CatalogNeed {
                    routes_by_catalog: false,
                    refresh: false
                },
                "{pin}"
            );
        }
    }

    #[test]
    fn an_unpinned_session_still_discovers_inline_and_refreshes() {
        for attended in [true, false] {
            let need = catalog_need(None, attended, false);
            assert!(
                need.routes_by_catalog && need.refresh,
                "attended={attended}"
            );
        }
    }

    #[test]
    fn a_pinned_tui_starts_at_once_and_refreshes_behind_the_first_frame() {
        let need = catalog_need(Some(GATEWAY), true, false);
        assert!(!need.routes_by_catalog);
        assert!(need.refresh);
    }

    #[test]
    fn a_pin_that_still_ranks_through_the_catalog_keeps_it() {
        let set = "inferhub::cb/deepseek-v4.1-flash,claude-cli::haiku";
        assert_eq!(
            catalog_need(Some(set), false, false),
            CatalogNeed {
                routes_by_catalog: true,
                refresh: true
            },
            "a set ranks its members through the catalog"
        );
        assert!(
            catalog_need(Some(GATEWAY), false, true).routes_by_catalog,
            "pin_failover falls back across the catalog"
        );
        assert!(
            catalog_need(Some("sonnet"), false, false).routes_by_catalog,
            "a bare alias is resolved through the catalog"
        );
    }
}
