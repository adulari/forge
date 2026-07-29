//! Command-line grammar for MCP client configuration and Forge MCP serving.

use super::Scope;
use clap::{Subcommand, ValueEnum};

#[derive(Subcommand)]
pub(crate) enum McpCmd {
    /// Add an MCP server to the config (compatible with `claude mcp add` / `codex mcp add`).
    ///
    /// Examples:
    ///   forge mcp add myserver -- npx -y @scope/mcp-server
    ///   forge mcp add myserver --transport http --url https://api.example.com/mcp
    ///   forge mcp add myserver -e API_KEY=secret -- node server.js
    Add {
        /// Unique name for the server.
        name: String,
        /// Transport protocol.
        #[arg(long, default_value = "stdio")]
        transport: McpTransportArg,
        /// Config scope: `local`/`project` → `.forge/mcp.toml`; `user` → `~/.config/forge/mcp.toml`.
        #[arg(long, short = 's', default_value = "local")]
        scope: Scope,
        /// Environment variables to pass to the stdio process (KEY=VALUE).
        #[arg(long, short = 'e', value_name = "KEY=VALUE")]
        env: Vec<String>,
        /// HTTP headers to add to requests (KEY=VALUE).
        #[arg(long, value_name = "KEY=VALUE")]
        header: Vec<String>,
        /// HTTP/SSE server URL (required for `--transport http` or `--transport sse`).
        #[arg(long)]
        url: Option<String>,
        /// Environment variable holding the bearer token for auth.
        #[arg(long, value_name = "ENV_VAR")]
        bearer_token_env_var: Option<String>,
        /// Command and arguments for stdio servers (everything after `--`).
        #[arg(last = true, value_name = "COMMAND")]
        command: Vec<String>,
    },
    /// Remove an MCP server from the config.
    Remove {
        /// Server name to remove.
        name: String,
        /// Config scope to remove from.
        #[arg(long, short = 's', default_value = "local")]
        scope: Scope,
    },
    /// Show the config entry for one MCP server.
    Get {
        /// Server name to look up.
        name: String,
    },
    /// Expose a persistent Forge session as an MCP server on stdio, so another agent
    /// (Claude Code, another Forge) can drive it via `forge_chat` / `forge_status` /
    /// `forge_set_mode`. Add to `.mcp.json`: `{"forge": {"type":"stdio","command":"forge","args":["mcp","agent"]}}`.
    Agent {
        /// Resume an existing session by ID prefix instead of starting a fresh one.
        #[arg(long)]
        session: Option<String>,
        /// Change the working directory before starting (the session's tool calls operate here).
        #[arg(long)]
        cwd: Option<std::path::PathBuf>,
    },
    /// Show the full discovered tool list for one connected server.
    Tools {
        /// Server name (as declared in `.forge/mcp.toml`).
        server: String,
    },
    /// Import a Claude-Code-style `.mcp.json` into `.forge/mcp.toml` (secrets are NOT copied).
    Import {
        /// Path to the `.mcp.json` (default: `./.mcp.json`).
        path: Option<String>,
    },
    /// Obtain OAuth tokens for an OAuth-protected HTTP MCP server.
    Login {
        /// Server name (as declared in `.forge/mcp.toml`).
        server: String,
        /// Force RFC 8628 device authorization when advertised.
        #[arg(long)]
        device: bool,
        /// Use pasted redirect (or read it from stdin when no value is given).
        #[arg(long, num_args = 0..=1, default_missing_value = "", value_name = "REDIRECT")]
        paste: Option<String>,
    },
    /// Remove stored OAuth tokens for a server (`forge mcp logout <server>`). Bare removes every
    /// account; `--account <id>` removes just one.
    Logout {
        /// Server name (as declared in `.forge/mcp.toml`).
        server: String,
        /// Remove just this account instead of every account stored for the server.
        #[arg(long)]
        account: Option<String>,
    },
}

#[derive(Clone, ValueEnum, Debug)]
pub(crate) enum McpTransportArg {
    Stdio,
    Sse,
    Http,
}

/// Transport for `forge mcp-serve` (Forge serving its own tools as an MCP **server**).
#[derive(Clone, ValueEnum, Debug)]
pub(crate) enum ServeTransportArg {
    Stdio,
    Http,
}
