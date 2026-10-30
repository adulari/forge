//! MCP servers switched off for one session (`POST /api/sessions` with `mcp_off`).
//!
//! Carried as a task-local around the session build instead of through every builder signature:
//! the build reads its MCP catalog inside the awaited future, so the scope reaches it.

use std::future::Future;

tokio::task_local! {
    static MCP_OFF: Vec<String>;
}

pub(crate) async fn scoped<F: Future>(off: Vec<String>, build: F) -> F::Output {
    MCP_OFF.scope(off, build).await
}

pub(crate) fn apply(mcp: &mut forge_config::McpConfig) {
    let _ = MCP_OFF.try_with(|off| mcp.servers.retain(|server| !off.contains(&server.name)));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(name: &str) -> forge_config::McpServerConfig {
        toml::from_str(&format!(
            "name = \"{name}\"\n[transport]\ntype = \"stdio\"\ncommand = \"x\"\n"
        ))
        .unwrap()
    }

    #[tokio::test]
    async fn only_the_scoped_build_drops_the_named_servers() {
        let config = forge_config::McpConfig {
            servers: vec![server("keep"), server("drop")],
            ..Default::default()
        };
        let mut outside = config.clone();
        apply(&mut outside);
        assert_eq!(outside.servers.len(), 2);
        let inside = scoped(vec!["drop".into()], async {
            let mut c = config.clone();
            apply(&mut c);
            c
        })
        .await;
        assert_eq!(inside.servers.len(), 1);
        assert_eq!(inside.servers[0].name, "keep");
    }
}
