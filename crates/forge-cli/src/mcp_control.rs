// ServerInfo is deprecated in rmcp 3 but still the type `ServerHandler::get_info` returns.
#![allow(deprecated)]
//! `forge mcp control` - the control toolset as a stdio MCP server, so another agent (Claude
//! Code, Codex, another Forge) can start sessions, read terminals and manage schedules in the
//! running Forge. Every call is forwarded to the daemon, which honours the per-tool disable list
//! and writes the audit line. The connecting client's own tool approval is the permission gate
//! here: Forge has no prompt to show on this transport.
//!
//! Example `.mcp.json`:
//! `{"forge-control": {"type": "stdio", "command": "forge", "args": ["mcp", "control"]}}`

use std::sync::Arc;

use anyhow::Result;
use forge_core::control;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::RequestContext;
use rmcp::transport::io::stdio;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler, ServiceExt};
use serde_json::Value;

use crate::control_http::HttpControl;

struct ControlServer {
    daemon: HttpControl,
}

fn tool_list() -> Vec<Tool> {
    control::specs(&crate::control_settings::disabled())
        .into_iter()
        .map(|spec| {
            let schema = spec.schema.as_object().cloned().unwrap_or_default();
            Tool::new(spec.name, spec.description, Arc::new(schema))
        })
        .collect()
}

impl ServerHandler for ControlServer {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.instructions = Some(
            "Drive the running Forge app: list projects, start sessions and worktrees, read a \
             session or its terminal, message a session, add schedules and MCP servers, check \
             usage, notify the user. Needs `forge serve` running. Tools the user switched off in \
             Settings are refused."
                .into(),
        );
        info
    }

    async fn list_tools(
        &self,
        _req: Option<PaginatedRequestParams>,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult::with_all_items(tool_list()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let name = request.name.to_string();
        let args = request.arguments.map(Value::Object).unwrap_or(Value::Null);
        let result = if let Err(error) = control::validate(&name, &args) {
            CallToolResult::error(vec![ContentBlock::text(error)])
        } else {
            match self.daemon.run("", &name, &args).await {
                Ok(text) => CallToolResult::success(vec![ContentBlock::text(text)]),
                Err(text) => CallToolResult::error(vec![ContentBlock::text(text)]),
            }
        };
        Ok(result.into())
    }
}

pub(crate) async fn serve() -> Result<()> {
    let server = ControlServer {
        daemon: HttpControl::local(),
    };
    let running = server.serve(stdio()).await?;
    running.waiting().await?;
    Ok(())
}

pub(crate) fn list() {
    let disabled = crate::control_settings::disabled();
    for tool in control::catalog(&disabled) {
        println!(
            "{:<18} {:<5} {}",
            tool.name,
            if tool.enabled { "on" } else { "off" },
            tool.kind
        );
    }
}

pub(crate) fn set(tool: &str, enabled: bool) -> Result<()> {
    crate::control_settings::set_enabled(tool, enabled).map_err(|e| anyhow::anyhow!(e))?;
    println!("{tool}: {}", if enabled { "on" } else { "off" });
    Ok(())
}

pub(crate) fn log(limit: usize) {
    let entries = crate::serve_control::recent_audit(limit);
    if entries.is_empty() {
        println!("no control tool calls recorded");
    }
    for e in entries.iter().rev() {
        println!(
            "{}  {:<5} {:<20} {}",
            e["ts"].as_str().unwrap_or("?"),
            e["decision"].as_str().unwrap_or("?"),
            e["route"]
                .as_str()
                .unwrap_or("?")
                .trim_start_matches("control/"),
            e["detail"].as_str().unwrap_or("")
        );
    }
}
