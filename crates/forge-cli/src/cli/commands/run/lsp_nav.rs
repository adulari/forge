//! Registers the `lsp` navigation tool (`[harness] lsp_nav`) and builds the language-server pool
//! it shares with post-edit diagnostics.

use std::sync::Arc;

use forge_config::Config;
use forge_tools::ToolRegistry;

/// The pool for this session: present when diagnostics or navigation is on. With `lsp_nav` the
/// `lsp` tool is registered (deferred, unless `mesh.lean_core_extra` promotes it); inlined library
/// source is checked against the session's `read_file` rules, since it is a read.
pub(super) fn build(
    config: &Config,
    tools: &mut ToolRegistry,
) -> Option<Arc<forge_lsp::LspRegistry>> {
    let nav = config.harness.lsp_nav;
    if !config.lsp.enabled && !nav {
        return None;
    }
    let registry = Arc::new(if nav {
        forge_lsp::LspRegistry::from_config_nav(&config.lsp)
    } else {
        forge_lsp::LspRegistry::from_config(&config.lsp)
    });
    if nav {
        let rules = config.permission_rules();
        forge_tools::lsp_tool::register(tools, Arc::clone(&registry), move |path| {
            let args = serde_json::json!({ "path": path.display().to_string() });
            forge_core::permission::denying_rule("read_file", &args, &rules).is_none()
        });
    }
    Some(registry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registers_only_when_the_flag_is_on() {
        let mut config = Config::default();
        let mut tools = ToolRegistry::with_core_tools();
        assert!(build(&config, &mut tools).is_none());
        assert!(tools.get("lsp").is_none());

        config.harness.lsp_nav = true;
        assert!(build(&config, &mut tools).is_some());
        assert!(tools.get("lsp").is_some());
    }

    #[test]
    fn diagnostics_alone_builds_the_pool_without_the_tool() {
        let mut config = Config::default();
        config.lsp.enabled = true;
        let mut tools = ToolRegistry::with_core_tools();
        assert!(build(&config, &mut tools).is_some());
        assert!(tools.get("lsp").is_none());
    }
}
