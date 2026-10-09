//! Startup connect failures are routine (the daemon starts before DNS works), so what is logged
//! for them is one short line per server, and one more only if the retries run out.

/// The transport error text with rmcp's type-path noise removed. A failed streamable-HTTP connect
/// renders as `Send message error Transport [rmcp::transport::worker::WorkerTransport<rmcp::...
/// <reqwest::async_impl::client::Client>>] error: Client error: error sending request for url
/// (...)`, of which only the last part helps anyone.
pub(crate) fn concise_reason(reason: &str) -> String {
    const MAX: usize = 220;
    let mut out = String::with_capacity(reason.len());
    let mut depth = 0usize;
    for c in reason.chars() {
        match c {
            '[' => depth += 1,
            ']' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    let out = out.replace("Transport  error", "transport error");
    if out.chars().count() > MAX {
        out.chars().take(MAX).collect::<String>() + "…"
    } else {
        out
    }
}

/// Log servers still failed after `rounds` retries (once, at the end) and return how many.
pub(crate) fn report_still_failed(
    servers: &[forge_config::McpServerConfig],
    rounds: usize,
) -> usize {
    if rounds > 0 && !servers.is_empty() {
        let names: Vec<&str> = servers.iter().map(|s| s.name.as_str()).collect();
        tracing::warn!(
            "mcp: still unreachable after {rounds} retry round(s): {} — left as failed",
            names.join(", ")
        );
    }
    servers.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{McpConfig, McpManager};
    use std::time::Duration;

    #[test]
    fn rmcp_type_paths_are_stripped_from_a_connect_failure() {
        let raw = "initialize: Send message error Transport [rmcp::transport::worker::WorkerTransport<rmcp::transport::streamable_http_client::StreamableHttpClientWorker<reqwest::async_impl::client::Client>>] error: Client error: error sending request for url (https://helm.adulari.dev/mcp), when send initialize request";
        let short = concise_reason(raw);
        assert!(!short.contains("rmcp::"), "{short}");
        assert!(short.contains("error sending request for url (https://helm.adulari.dev/mcp)"));
        assert!(short.len() < raw.len() * 3 / 4, "{short}");
    }

    #[test]
    fn plain_reasons_and_long_ones_are_kept_readable() {
        assert_eq!(concise_reason("connection refused"), "connection refused");
        assert!(concise_reason(&"x".repeat(1_000)).chars().count() <= 221);
    }

    /// A stdio MCP server that exits until `marker` exists, then answers the handshake: the shape
    /// of a remote server that is unreachable at login and fine a few seconds later.
    fn flaky_server(marker: &std::path::Path) -> forge_config::McpServerConfig {
        let script = r#"
import json, sys
for line in sys.stdin:
    msg = json.loads(line)
    if "id" not in msg:
        continue
    if msg["method"] == "initialize":
        result = {"protocolVersion": msg["params"]["protocolVersion"],
                  "capabilities": {"tools": {}},
                  "serverInfo": {"name": "flaky", "version": "1"}}
    else:
        result = {"tools": []}
    print(json.dumps({"jsonrpc": "2.0", "id": msg["id"], "result": result}), flush=True)
"#;
        forge_config::McpServerConfig {
            name: "flaky".into(),
            transport: forge_config::McpTransport::Stdio {
                command: "sh".into(),
                args: vec![
                    "-c".into(),
                    format!(
                        "test -f '{}' || exit 1; exec python3 -c '{script}'",
                        marker.display()
                    ),
                ],
                env: Default::default(),
            },
            auth: None,
            secret_env: vec![],
            enabled: true,
        }
    }

    #[tokio::test]
    async fn a_server_that_was_down_at_startup_connects_on_retry() {
        let marker = std::env::temp_dir().join(format!("forge-mcp-flaky-{}", std::process::id()));
        let _ = std::fs::remove_file(&marker);
        let config = McpConfig {
            connect_timeout_secs: 10,
            servers: vec![flaky_server(&marker)],
            ..Default::default()
        };
        let mgr = McpManager::connecting(&config);
        mgr.connect_active().await;
        assert_eq!(mgr.failed_servers().len(), 1, "down at startup");

        std::fs::write(&marker, b"up").unwrap();
        let left = mgr
            .retry_failed_connects(&[Duration::from_millis(1)], Duration::from_millis(1), 2)
            .await;
        let _ = std::fs::remove_file(&marker);
        assert_eq!(left, 0, "the retry brings it up");
        assert!(mgr.failed_servers().is_empty());
    }
}
