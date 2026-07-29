# Code archaeology: MCP command grammar

## Boundary

`cli/args/mcp.rs` owns the complete Clap grammar for configuring external MCP servers and selecting Forge's own MCP server transport. Runtime connection, OAuth, import, and mutation logic remain in MCP command handlers.

## Interface

`McpCmd`, `McpTransportArg`, and `ServeTransportArg` are the parser/dispatch contract. The `add` scope defaults, `--` command tail, transport values, and login redirect option are CLI compatibility surface.

## Characterization

The existing MCP scope parser tests exercise nested `mcp add` parsing through the root `Cli`; MCP command handler tests cover transport and OAuth behavior. The extraction preserves the existing enum values and root re-exports.
