//! Spots a credential written straight into an MCP server definition. Definitions travel as plain
//! (not vault-sealed) records to every device with `mcp` on, so one that carries a secret must stay
//! local; the keyring-backed `keys` category is the only channel allowed to move credentials.
//!
//! This is a heuristic with one bias: when unsure, call it a secret. A wrongly held-back server is
//! reported as skipped and costs one manual copy; a leaked credential cannot be taken back.

use forge_config::mcp::{McpServerConfig, McpTransport};

const REASON: &str = "holds a credential inline; move it to the keyring (`forge mcp`)";

/// Parts of a flag, variable or query-parameter name that mean its value is a credential. Compared
/// lowercase with `_` folded to `-`. Plain `auth` and `key` are left out (`--author`, `--key-file`
/// would match); `looks_secret` still covers the broad env-var-name case.
const SECRET_NAME_PARTS: [&str; 10] = [
    "token",
    "secret",
    "password",
    "passwd",
    "api-key",
    "apikey",
    "access-key",
    "private-key",
    "credential",
    "authorization",
];

/// Query parameters that carry a credential in practice although the name alone is generic.
const SECRET_QUERY_NAMES: [&str; 6] = ["key", "sig", "signature", "auth", "code", "pat"];

const TOKEN_PREFIXES: [&str; 22] = [
    "sk-",
    "sk_",
    "pk_live",
    "rk_live",
    "ghp_",
    "gho_",
    "ghs_",
    "ghu_",
    "ghr_",
    "github_pat_",
    "glpat-",
    "xoxb-",
    "xoxp-",
    "xoxa-",
    "xapp-",
    "AKIA",
    "ASIA",
    "AIza",
    "eyJ",
    "npm_",
    "hf_",
    "ya29.",
];

pub(super) fn embedded_secret(server: &McpServerConfig) -> Option<String> {
    let found = match &server.transport {
        McpTransport::Stdio { args, env, .. } => {
            env.iter().any(|(name, value)| {
                is_set(value)
                    && (forge_config::looks_secret(name)
                        || forge_config::embeds_credentials(value)
                        || looks_like_token(value)
                        || value_has_url_secret(value))
            }) || args_hold_secret(args)
        }
        McpTransport::Http { url, headers } | McpTransport::Sse { url, headers } => {
            headers.iter().any(|(name, value)| {
                is_set(value)
                    && (forge_config::looks_secret(name)
                        || name.eq_ignore_ascii_case("authorization")
                        || looks_like_token(value)
                        || value.to_ascii_lowercase().starts_with("bearer "))
            }) || url_holds_secret(url)
        }
    };
    found.then(|| REASON.to_string())
}

fn is_set(value: &str) -> bool {
    !value.is_empty() && !value.starts_with("${")
}

fn normalized(name: &str) -> String {
    name.to_ascii_lowercase().replace('_', "-")
}

fn secret_name(name: &str) -> bool {
    let name = normalized(name);
    SECRET_NAME_PARTS.iter().any(|part| name.contains(part))
}

fn looks_like_token(value: &str) -> bool {
    TOKEN_PREFIXES
        .iter()
        .any(|prefix| value.starts_with(prefix))
}

fn value_has_url_secret(value: &str) -> bool {
    value.contains("://") && url_holds_secret(value)
}

fn args_hold_secret(args: &[String]) -> bool {
    args.iter().enumerate().any(|(index, arg)| {
        let lower = arg.to_ascii_lowercase();
        if forge_config::embeds_credentials(arg)
            || looks_like_token(arg)
            || lower.contains("authorization:")
            || lower.contains("bearer ")
            || value_has_url_secret(arg)
        {
            return true;
        }
        let bare = arg.trim_start_matches('-');
        match bare.split_once('=') {
            Some((name, value)) => secret_name(name) && is_set(value),
            None => {
                arg.starts_with('-')
                    && secret_name(bare)
                    && args
                        .get(index + 1)
                        .is_some_and(|next| !next.starts_with('-') && is_set(next))
            }
        }
    })
}

fn url_holds_secret(url: &str) -> bool {
    if forge_config::embeds_credentials(url) {
        return true;
    }
    let Some((_, rest)) = url.split_once("://") else {
        return false;
    };
    let rest = rest.split('#').next().unwrap_or("");
    let (location, query) = rest.split_once('?').unwrap_or((rest, ""));
    let path_secret = location.split('/').skip(1).any(token_like_segment);
    let query_secret = query.split('&').any(|pair| {
        let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
        let folded = normalized(name);
        is_set(value) && (secret_name(name) || SECRET_QUERY_NAMES.contains(&folded.as_str()))
    });
    path_secret || query_secret
}

/// A long run of letters and digits in a URL path is an access token far more often than a slug.
fn token_like_segment(segment: &str) -> bool {
    segment.len() >= 20
        && !segment.starts_with("${")
        && segment
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        && segment.chars().any(|c| c.is_ascii_digit())
        && segment.chars().any(|c| c.is_ascii_alphabetic())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(transport: serde_json::Value) -> McpServerConfig {
        serde_json::from_value(serde_json::json!({ "name": "s", "transport": transport })).unwrap()
    }

    fn stdio(args: &[&str], env: &[(&str, &str)]) -> McpServerConfig {
        server(serde_json::json!({
            "type": "stdio",
            "command": "npx",
            "args": args,
            "env": env.iter().cloned().collect::<std::collections::BTreeMap<_, _>>(),
        }))
    }

    fn http(url: &str) -> McpServerConfig {
        server(serde_json::json!({ "type": "http", "url": url }))
    }

    #[test]
    fn credentials_hiding_in_env_values_args_and_urls_are_caught() {
        let secret = [
            stdio(&[], &[("DATABASE_URL", "postgres://app:hunter2@db/prod")]),
            stdio(
                &[],
                &[("UPSTREAM", "ghp_abcdefghijklmnopqrstuvwxyz0123456789")],
            ),
            stdio(&[], &[("TARGET", "https://x.example/sse?api_key=abc123")]),
            stdio(&["--api-key", "abcd1234"], &[]),
            stdio(&["--token=abcd1234"], &[]),
            stdio(&["--auth-token", "abcd1234"], &[]),
            stdio(&["-e", "SERVICE_PASSWORD=hunter2"], &[]),
            stdio(&["-H", "Authorization: Bearer abc"], &[]),
            stdio(&["sk-live-abcdefghijklmnop"], &[]),
            stdio(&["--url", "https://x.example/mcp?token=abc"], &[]),
            http("https://mcp.zapier.com/api/mcp/s/Zm9vYmFyMTIzNDU2Nzg5MDEyMw/mcp"),
            http("https://x.example/sse?api_key=abc123"),
            http("https://x.example/sse?auth=abc123&v=1"),
            http("https://app:pw@x.example/mcp"),
            server(serde_json::json!({
                "type": "http", "url": "https://x.example/mcp",
                "headers": { "X-Custom": "Bearer abcdef" },
            })),
        ];
        for (index, definition) in secret.iter().enumerate() {
            assert!(
                embedded_secret(definition).is_some(),
                "case {index} slipped through: {definition:?}"
            );
        }
    }

    #[test]
    fn ordinary_definitions_and_references_are_not_held_back() {
        let plain = [
            stdio(
                &["-y", "@modelcontextprotocol/server-filesystem", "/home/u"],
                &[],
            ),
            stdio(
                &["--path", "/srv", "--port", "8080"],
                &[("LOG_LEVEL", "debug")],
            ),
            stdio(&["--author", "me", "--key-file", "/x"], &[]),
            stdio(&["--api-key", "${MY_KEY}"], &[("TOKEN", "${MY_TOKEN}")]),
            http("https://example.com/mcp"),
            http("https://example.com/v1/docs/model-context-protocol-guide"),
            http("https://example.com/mcp?version=2&format=json"),
            server(serde_json::json!({
                "type": "http", "url": "https://x.example/mcp",
                "headers": { "Authorization": "${AUTH_HEADER}" },
            })),
        ];
        for (index, definition) in plain.iter().enumerate() {
            assert!(
                embedded_secret(definition).is_none(),
                "case {index} was held back: {definition:?}"
            );
        }
    }
}
