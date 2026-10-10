//! Test/wiring support — not part of the stable API. A tiny in-process MCP server (two tools:
//! `echo`, `boom`) served over a duplex stream, plus [`manager_with_echo`] which returns an
//! [`McpManager`] connected to it. Lets downstream crates (forge-core, forge-cli) exercise their
//! MCP integration against a real connection without spawning a child process.

use super::*;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler, ServiceExt};
use std::sync::Arc;

#[derive(Clone)]
struct EchoServer;

impl ServerHandler for EchoServer {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info
    }
    async fn list_tools(
        &self,
        _req: Option<PaginatedRequestParams>,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let schema: rmcp::model::JsonObject = serde_json::from_value(serde_json::json!({
            "type": "object",
            "properties": { "msg": { "type": "string" } }
        }))
        .unwrap();
        // `with_all_items` fills rmcp 3's new paginated-result fields with spec defaults.
        Ok(ListToolsResult::with_all_items(vec![
            Tool::new(
                "echo",
                "Echo back the msg argument",
                Arc::new(schema.clone()),
            ),
            Tool::new("boom", "Always fails", Arc::new(schema)),
        ]))
    }
    async fn call_tool(
        &self,
        req: CallToolRequestParams,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        // This mock always completes; `.into()` wraps it as CallToolResponse::Complete.
        match req.name.as_ref() {
            "echo" => {
                let msg = req
                    .arguments
                    .as_ref()
                    .and_then(|a| a.get("msg"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                Ok(
                    CallToolResult::success(vec![ContentBlock::text(format!("echo: {msg}"))])
                        .into(),
                )
            }
            "boom" => Ok(CallToolResult::error(vec![ContentBlock::text("kaboom")]).into()),
            other => Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "unknown tool {other}"
            ))])
            .into()),
        }
    }
}

/// An in-process echo server and the client end of it, with the means to kill the server and
/// to see that it has gone (the connection was closed from the client side).
#[cfg(test)]
pub(crate) struct EchoServerHandle {
    pub established: transport::Established,
    pub kill: tokio::sync::oneshot::Sender<()>,
    pub closed: Arc<std::sync::atomic::AtomicBool>,
}

#[cfg(test)]
pub(crate) async fn spawn_echo(name: &str) -> EchoServerHandle {
    let (client_io, server_io) = tokio::io::duplex(8 * 1024);
    let (kill, killed) = tokio::sync::oneshot::channel::<()>();
    let closed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = Arc::clone(&closed);
    tokio::spawn(async move {
        if let Ok(server) = EchoServer.serve(server_io).await {
            tokio::select! {
                _ = server.waiting() => {}
                _ = killed => {}
            }
        }
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
    });
    let client = ForgeClientHandler::passive(name)
        .serve(client_io)
        .await
        .expect("client connects");
    EchoServerHandle {
        established: transport::Established {
            service: client,
            child_group: None,
        },
        kill,
        closed,
    }
}

/// An [`McpManager`] connected (in-process) to a server named `test` exposing `echo`+`boom`.
pub async fn manager_with_echo(config: &McpConfig) -> McpManager {
    let (client_io, server_io) = tokio::io::duplex(8 * 1024);
    tokio::spawn(async move {
        if let Ok(server) = EchoServer.serve(server_io).await {
            let _ = server.waiting().await;
        }
    });
    let client = ForgeClientHandler::passive("test")
        .serve(client_io)
        .await
        .expect("client connects");
    let mgr = McpManager::empty(config);
    mgr.add_established(
        "test",
        "stdio",
        transport::Established {
            service: client,
            child_group: None,
        },
    )
    .await;
    mgr
}
