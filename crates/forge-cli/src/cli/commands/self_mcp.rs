use anyhow::Result;

use crate::SelfMcpAction;

pub fn self_mcp_cmd(action: SelfMcpAction) -> Result<()> {
    match action {
        SelfMcpAction::Enable => {
            let path = forge_config::write_self_mcp(true)
                .map_err(|e| anyhow::anyhow!("failed to write config: {e}"))?;
            println!(
                "self-MCP enabled — forge_chat / forge_assay / forge_interrupt available as \
                 tools on next session start\nconfig: {}",
                path.display()
            );
        }
        SelfMcpAction::Disable => {
            let path = forge_config::write_self_mcp(false)
                .map_err(|e| anyhow::anyhow!("failed to write config: {e}"))?;
            println!(
                "self-MCP disabled — sub-Forge MCP server will not start on next session\n\
                 config: {}",
                path.display()
            );
        }
        SelfMcpAction::Status => {
            let enabled = forge_config::load().map(|c| c.self_mcp).unwrap_or(false);
            println!("self-MCP: {}", if enabled { "enabled" } else { "disabled" });
        }
    }
    Ok(())
}
