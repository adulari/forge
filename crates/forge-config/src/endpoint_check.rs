//! Shape checks for user-supplied provider endpoints and API-key env var names.

/// The daemon API only accepts `[A-Z_][A-Z0-9_]*` for an API-key env var name; enforce the same on
/// the CLI write path so the two surfaces agree. Empty / absent means keyless.
pub(crate) fn check_env_var_name(name: Option<&str>) -> Result<(), String> {
    let Some(name) = name.map(str::trim).filter(|n| !n.is_empty()) else {
        return Ok(());
    };
    let mut chars = name.chars();
    let first_ok = chars
        .next()
        .is_some_and(|c| c.is_ascii_uppercase() || c == '_');
    if first_ok && chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') {
        Ok(())
    } else {
        Err("api_key_env must use uppercase ASCII letters, digits, and underscores".to_string())
    }
}

/// Why `endpoint` (already `http(s)://`-prefixed) is not a usable base URL, if it is not: an empty
/// host, whitespace, embedded `user:pass@` credentials (they would be stored and echoed), or a bad
/// port. The message never repeats the endpoint, since it may carry a secret.
fn shape_problem(endpoint: &str) -> Option<&'static str> {
    let rest = endpoint.split_once("://").map_or(endpoint, |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if endpoint.chars().any(char::is_whitespace) {
        return Some("must not contain whitespace");
    }
    if authority.contains('@') {
        return Some("must not embed credentials (user:pass@) — use api_key_env");
    }
    let (host, port) = if let Some(v6) = authority.strip_prefix('[') {
        let (h, tail) = v6.split_once(']').unwrap_or((v6, ""));
        (h, tail.strip_prefix(':'))
    } else {
        match authority.rsplit_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (authority, None),
        }
    };
    if host.is_empty() {
        return Some("has no host");
    }
    if port.is_some_and(|p| p.parse::<u16>().is_err()) {
        return Some("has an invalid port");
    }
    None
}

/// Reject a malformed endpoint with a message that never repeats it. `require_https` also refuses
/// plain `http://` except to loopback (the API key travels in a header).
pub(crate) fn endpoint_shape_error(
    what: &str,
    endpoint: &str,
    require_https: bool,
) -> Result<(), String> {
    if let Some(why) = shape_problem(endpoint) {
        return Err(format!("{what} {why}"));
    }
    let loopback = ["//localhost", "//127.0.0.1", "//[::1]"]
        .iter()
        .any(|h| endpoint.contains(h));
    if require_https && endpoint.starts_with("http://") && !loopback {
        return Err(format!(
            "{what} must be https:// (the key is sent in a header)"
        ));
    }
    Ok(())
}

/// Namespaces owned by first-class providers (OAuth, CLI bridges, local runtime): a custom
/// endpoint under one would inject models into the real provider's catalog.
pub(crate) fn is_reserved_namespace(ns: &str) -> bool {
    ns == "ollama" || ns.ends_with("-cli") || ns.ends_with("-oauth")
}
