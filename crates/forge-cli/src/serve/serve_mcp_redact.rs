//! Masking for the MCP listing shown to callers that may not read secrets.
//!
//! Servers keep tokens in their argv (`--access-token sbp_...`) and URLs (`.../s/<secret>/sse`,
//! `?api_key=`, `user:pass@host`). `GET /api/mcp` is a View-scope read that also crosses the
//! Anywhere relay, so the listing masks anything that looks like a credential. Callers holding the
//! config-write scope on a direct connection get the values untouched.

const MASK: &str = "***";
const SECRET_WORDS: [&str; 10] = [
    "token",
    "secret",
    "password",
    "passwd",
    "pwd",
    "apikey",
    "api_key",
    "api-key",
    "auth",
    "credential",
];
const TOKEN_PREFIXES: [&str; 14] = [
    "sbp_",
    "ghp_",
    "gho_",
    "ghs_",
    "github_pat_",
    "glpat-",
    "sk-",
    "xox",
    "akia",
    "eyj",
    "bearer ",
    "pat_",
    "shpat_",
    "key-",
];

fn secret_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    SECRET_WORDS.iter().any(|word| lower.contains(word))
        || lower == "key"
        || lower.ends_with("_key")
        || lower.ends_with("-key")
        || lower.ends_with("key") && lower.len() > 3 && name.chars().any(char::is_uppercase)
}

fn secret_value(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    if TOKEN_PREFIXES
        .iter()
        .any(|prefix| lower.starts_with(prefix))
    {
        return true;
    }
    value.len() >= 24
        && !value.contains(['/', ' ', '.', '\\'])
        && value.chars().any(|c| c.is_ascii_digit())
        && value.chars().any(|c| c.is_ascii_alphabetic())
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '+' | '='))
}

fn redact_segment(segment: &str) -> String {
    if secret_value(segment) {
        MASK.to_string()
    } else {
        segment.to_string()
    }
}

pub(super) fn redact_url(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return redact_segment(url);
    };
    let (rest, fragment) = match rest.split_once('#') {
        Some((rest, _)) => (rest, Some(MASK)),
        None => (rest, None),
    };
    let (rest, query) = match rest.split_once('?') {
        Some((rest, query)) => (rest, Some(query)),
        None => (rest, None),
    };
    let (authority, path) = match rest.find('/') {
        Some(at) => (&rest[..at], &rest[at..]),
        None => (rest, ""),
    };
    let authority = match authority.rsplit_once('@') {
        Some((_, host)) => format!("{MASK}@{host}"),
        None => authority.to_string(),
    };
    let path = path
        .split('/')
        .map(redact_segment)
        .collect::<Vec<_>>()
        .join("/");
    let mut out = format!("{scheme}://{authority}{path}");
    if let Some(query) = query {
        let pairs: Vec<String> = query
            .split('&')
            .map(|pair| match pair.split_once('=') {
                Some((key, value)) if secret_name(key) || secret_value(value) => {
                    format!("{key}={MASK}")
                }
                _ => pair.to_string(),
            })
            .collect();
        out.push('?');
        out.push_str(&pairs.join("&"));
    }
    if fragment.is_some() {
        out.push('#');
        out.push_str(MASK);
    }
    out
}

fn redact_one(arg: &str) -> String {
    if arg.contains("://") {
        return redact_url(arg);
    }
    if let Some((name, value)) = arg.split_once('=') {
        let name_part = name.trim_start_matches('-');
        if secret_name(name_part) || secret_value(value) {
            return format!("{name}={MASK}");
        }
        return arg.to_string();
    }
    redact_segment(arg)
}

pub(super) fn redact_args(args: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len());
    let mut mask_next = false;
    for arg in args {
        if std::mem::take(&mut mask_next) && !arg.starts_with('-') {
            out.push(MASK.to_string());
            continue;
        }
        if arg.starts_with('-') && !arg.contains('=') && secret_name(arg.trim_start_matches('-')) {
            mask_next = true;
        }
        out.push(redact_one(arg));
    }
    out
}

pub(super) fn redact_command(command: &str) -> String {
    redact_one(command)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|a| a.to_string()).collect()
    }

    #[test]
    fn argv_secrets_are_masked_and_ordinary_args_survive() {
        let out = redact_args(&args(&[
            "-y",
            "@supabase/mcp-server-supabase",
            "--access-token",
            "sbp_0123456789abcdef",
            "--api-key=hunter2hunter2",
            "--project-ref",
            "abc",
            "TOKEN=xyz",
            "https://user:pw@host.test/mcp?api_key=k1&x=1",
        ]));
        assert_eq!(
            out,
            [
                "-y",
                "@supabase/mcp-server-supabase",
                "--access-token",
                "***",
                "--api-key=***",
                "--project-ref",
                "abc",
                "TOKEN=***",
                "https://***@host.test/mcp?api_key=***&x=1",
            ]
        );
    }

    #[test]
    fn url_userinfo_query_and_path_secrets_are_masked() {
        assert_eq!(
            redact_url("https://mcp.zapier.com/api/mcp/s/AbCdEf0123456789AbCdEf0123456789/sse"),
            "https://mcp.zapier.com/api/mcp/s/***/sse"
        );
        assert_eq!(
            redact_url("https://u:p@h.test:8080/mcp?token=abc&v=2#frag"),
            "https://***@h.test:8080/mcp?token=***&v=2#***"
        );
        assert_eq!(
            redact_url("http://127.0.0.1:3000/mcp"),
            "http://127.0.0.1:3000/mcp"
        );
    }
}
