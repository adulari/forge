//! `forge mcp add` and `forge mcp add-json`: write one server into a scope's `mcp.toml`.
//!
//! The grammar is `claude mcp add`'s, so a command copied from a server's README works as is:
//! `add [options] <name> <commandOrUrl> [args...]`, `-t/--transport`, `-H "Name: value"`, and
//! `-- <command>` for a stdio command whose arguments look like flags. Forge's older `--url`
//! spelling still works.

use anyhow::{Context, Result};

use super::{finish_import, mcp_scope_path, validate_server_name};
use crate::{McpTransportArg, Scope};

/// One `forge mcp add` invocation, as parsed.
pub(crate) struct AddRequest {
    pub(crate) name: String,
    pub(crate) transport: Option<McpTransportArg>,
    pub(crate) scope: Scope,
    pub(crate) env: Vec<String>,
    pub(crate) headers: Vec<String>,
    pub(crate) url: Option<String>,
    pub(crate) bearer_token_env_var: Option<String>,
    pub(crate) command: Vec<String>,
}

fn is_url(target: &str) -> bool {
    target.starts_with("https://") || target.starts_with("http://")
}

/// The transport to use: the explicit one, else `http` for a URL target, else `stdio`. Claude Code
/// defaults to stdio even for a URL and then fails to spawn it; nobody means that.
pub(crate) fn effective_transport(
    explicit: Option<McpTransportArg>,
    url: Option<&str>,
    command: &[String],
) -> McpTransportArg {
    if let Some(transport) = explicit {
        return transport;
    }
    let url_target = url.is_some() || matches!(command, [only] if is_url(only));
    if url_target {
        McpTransportArg::Http
    } else {
        McpTransportArg::Stdio
    }
}

/// The URL of an http/sse server: `--url` or the positional target, never both and nothing more.
pub(crate) fn remote_url(url: Option<String>, command: Vec<String>) -> Result<String> {
    match (url, command.as_slice()) {
        (Some(url), []) => Ok(url),
        (None, [target]) => Ok(target.clone()),
        (None, []) => anyhow::bail!("an http/sse server needs a URL: `forge mcp add <name> <url>`"),
        (Some(_), [_, ..]) => {
            anyhow::bail!("give the URL once: either positionally or with --url, not both")
        }
        (None, [_, extra @ ..]) => anyhow::bail!(
            "an http/sse server takes one URL; unexpected extra argument(s): {}",
            extra.join(" ")
        ),
    }
}

/// A header as `Name: value` (Claude Code's form) or `Name=value` (Forge's older form). The first
/// separator wins, so `Authorization: Bearer abc==` keeps its `=` padding.
pub(crate) fn parse_header(raw: &str) -> Result<(String, String)> {
    let at = match (raw.find(':'), raw.find('=')) {
        (Some(c), Some(e)) => c.min(e),
        (Some(c), None) => c,
        (None, Some(e)) => e,
        (None, None) => {
            anyhow::bail!("--header must be \"Name: value\" or Name=value, got: {raw}")
        }
    };
    let (name, value) = (raw[..at].trim(), raw[at + 1..].trim());
    if name.is_empty() {
        anyhow::bail!("--header has no name: {raw}");
    }
    Ok((name.to_string(), value.to_string()))
}

/// `forge mcp add`: adds a server entry to the chosen scope's `mcp.toml`. Secrets passed by
/// env-var name stay references (ADR-0007); a literal secret in `-e`/`-H` is written as given,
/// with a warning.
pub(crate) fn mcp_add(req: AddRequest) -> Result<()> {
    let path = mcp_scope_path(&req.scope)?;
    validate_server_name(&req.name)?;

    let mut env_map = std::collections::HashMap::new();
    for kv in &req.env {
        let (k, v) = kv
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("--env must be KEY=VALUE, got: {kv}"))?;
        env_map.insert(k.to_string(), v.to_string());
    }
    let mut header_map = std::collections::HashMap::new();
    for h in &req.headers {
        let (k, v) = parse_header(h)?;
        header_map.insert(k, v);
    }
    for (k, v) in env_map.iter().chain(header_map.iter()) {
        if !v.is_empty()
            && (forge_config::looks_secret(k)
                || k.eq_ignore_ascii_case("authorization")
                || forge_config::embeds_credentials(v))
        {
            eprintln!(
                "warning: '{k}' looks like a secret and is written in plaintext to {}; prefer --bearer-token-env-var or `forge mcp add-json`, which store it in the keyring",
                path.display()
            );
        }
    }

    let transport = effective_transport(req.transport, req.url.as_deref(), &req.command);
    let mcp_transport = match transport {
        McpTransportArg::Stdio => {
            if req.url.is_some() {
                anyhow::bail!("--url needs --transport http or sse");
            }
            let (cmd, args) = req.command.split_first().ok_or_else(|| {
                anyhow::anyhow!(
                    "a stdio server needs a command: `forge mcp add <name> -- <command> [args...]`"
                )
            })?;
            forge_config::McpTransport::Stdio {
                command: cmd.clone(),
                args: args.to_vec(),
                env: env_map,
            }
        }
        McpTransportArg::Http => forge_config::McpTransport::Http {
            url: remote_url(req.url, req.command)?,
            headers: header_map,
        },
        // Legacy HTTP+SSE drives forge-mcp's own SSE client, distinct from streamable HTTP, so an
        // SSE-only server connects instead of silently falling back.
        McpTransportArg::Sse => forge_config::McpTransport::Sse {
            url: remote_url(req.url, req.command)?,
            headers: header_map,
        },
    };

    let auth = req
        .bearer_token_env_var
        .map(|env_var| forge_config::McpAuth {
            token_env: Some(env_var),
            token_keyring: None,
            header: None,
            oauth: None,
        });
    let server = forge_config::McpServerConfig {
        name: req.name.clone(),
        transport: mcp_transport,
        auth,
        secret_env: vec![],
        enabled: true,
        shared: None,
    };

    let mut config = forge_config::load_mcp_toml(&path);
    if config.servers.iter().any(|s| s.name == req.name) {
        anyhow::bail!(
            "server '{}' already exists in {}. Use `forge mcp remove {} --scope {}` first.",
            req.name,
            path.display(),
            req.name,
            req.scope.as_str()
        );
    }
    config.servers.push(server);
    forge_config::write_mcp_toml(&path, &config)
        .with_context(|| format!("writing {}", path.display()))?;
    println!("✓ added server '{}' to {}", req.name, path.display());
    Ok(())
}

/// `forge mcp add-json <name> '<json>'`: the same JSON shape `.mcp.json` uses, parsed by the
/// importer so a token in it lands in the keyring rather than the TOML.
pub(crate) fn mcp_add_json(name: &str, json: &str, scope: Scope) -> Result<()> {
    validate_server_name(name)?;
    let path = mcp_scope_path(&scope)?;
    let parsed = forge_config::mcp::import_mcp_server_json(name, json)?;
    if parsed.servers.is_empty() {
        anyhow::bail!(
            "the JSON is neither a stdio server (`command`) nor a remote one (`url`){}",
            parsed
                .notes
                .first()
                .map(|n| format!(": {n}"))
                .unwrap_or_default()
        );
    }
    if forge_config::load_mcp_toml(&path)
        .servers
        .iter()
        .any(|s| s.name == name)
    {
        anyhow::bail!(
            "server '{name}' already exists in {}. Use `forge mcp remove {name} --scope {}` first.",
            path.display(),
            scope.as_str()
        );
    }
    finish_import(&path, parsed.servers, parsed.secrets)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_url_target_means_http_unless_a_transport_is_named() {
        let url = args(&["https://mcp.sentry.dev/mcp"]);
        assert!(matches!(
            effective_transport(None, None, &url),
            McpTransportArg::Http
        ));
        assert!(matches!(
            effective_transport(Some(McpTransportArg::Sse), None, &url),
            McpTransportArg::Sse
        ));
        assert!(matches!(
            effective_transport(None, Some("https://x"), &[]),
            McpTransportArg::Http
        ));
        assert!(matches!(
            effective_transport(None, None, &args(&["npx", "server"])),
            McpTransportArg::Stdio
        ));
        // A command that merely mentions a URL as an argument stays stdio.
        assert!(matches!(
            effective_transport(None, None, &args(&["proxy", "https://x"])),
            McpTransportArg::Stdio
        ));
    }

    #[test]
    fn the_remote_url_comes_from_exactly_one_place() {
        assert_eq!(remote_url(None, args(&["https://a"])).unwrap(), "https://a");
        assert_eq!(
            remote_url(Some("https://b".into()), vec![]).unwrap(),
            "https://b"
        );
        assert!(remote_url(None, vec![]).is_err());
        assert!(remote_url(Some("https://b".into()), args(&["https://a"])).is_err());
        assert!(remote_url(None, args(&["https://a", "extra"])).is_err());
    }

    #[test]
    fn headers_take_both_the_claude_and_the_forge_form() {
        assert_eq!(
            parse_header("X-Api-Key: abc123").unwrap(),
            ("X-Api-Key".into(), "abc123".into())
        );
        assert_eq!(
            parse_header("Authorization: Bearer abc==").unwrap(),
            ("Authorization".into(), "Bearer abc==".into())
        );
        assert_eq!(
            parse_header("X-Url=http://a:1").unwrap(),
            ("X-Url".into(), "http://a:1".into())
        );
        assert!(parse_header("no-separator").is_err());
        assert!(parse_header(": value").is_err());
    }
}
