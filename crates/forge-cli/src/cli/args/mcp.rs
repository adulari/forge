//! Command-line grammar for MCP client configuration and Forge MCP serving.

use super::Scope;
use clap::{Subcommand, ValueEnum};

// Declared here, not in args.rs, to keep that file under its size ratchet.
#[path = "serve.rs"]
pub(crate) mod serve;

#[derive(Subcommand)]
pub(crate) enum McpCmd {
    /// Add an MCP server to the config. Takes `claude mcp add` syntax unchanged:
    /// `add [options] <name> <commandOrUrl> [args...]` (and `codex mcp add`'s `-- <command>`).
    ///
    /// Examples:
    ///   forge mcp add sentry --transport http https://mcp.sentry.dev/mcp
    ///   forge mcp add corridor -t http https://app.corridor.dev/api/mcp -H "Authorization: Bearer ..."
    ///   forge mcp add myserver -e API_KEY=secret -- npx -y @scope/mcp-server
    ///   forge mcp add myserver node server.js
    Add {
        /// Unique name for the server.
        name: String,
        /// Transport protocol. Defaults to `stdio`, or `http` when the target is an http(s) URL.
        #[arg(long, short = 't')]
        transport: Option<McpTransportArg>,
        /// Config scope: `local`/`project` → `.forge/mcp.toml`; `user` → `~/.config/forge/mcp.toml`.
        #[arg(long, short = 's', default_value = "local")]
        scope: Scope,
        /// Environment variables to pass to the stdio process (KEY=VALUE).
        #[arg(long, short = 'e', value_name = "KEY=VALUE")]
        env: Vec<String>,
        /// HTTP headers to add to requests (`"Name: value"` or `Name=value`).
        #[arg(long, short = 'H', value_name = "HEADER")]
        header: Vec<String>,
        /// HTTP/SSE server URL; the positional URL form is the same thing.
        #[arg(long)]
        url: Option<String>,
        /// Environment variable holding the bearer token for auth.
        #[arg(long, value_name = "ENV_VAR")]
        bearer_token_env_var: Option<String>,
        /// The server URL (http/sse), or the stdio command and its arguments. Put a command whose
        /// arguments start with `-` after `--`.
        #[arg(value_name = "COMMAND_OR_URL")]
        command: Vec<String>,
    },
    /// Add an MCP server from a JSON object, as `claude mcp add-json` does
    /// (e.g. `'{"type":"http","url":"https://...","headers":{...}}'`). Secrets go to the keyring.
    AddJson {
        /// Unique name for the server.
        name: String,
        /// The server's JSON config (`command`/`args`/`env`, or `type` + `url`/`headers`).
        json: String,
        /// Config scope: `local`/`project` → `.forge/mcp.toml`; `user` → `~/.config/forge/mcp.toml`.
        #[arg(long, short = 's', default_value = "local")]
        scope: Scope,
    },
    /// List the configured MCP servers and their connection status (same as bare `forge mcp`).
    #[command(visible_alias = "ls")]
    List,
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
