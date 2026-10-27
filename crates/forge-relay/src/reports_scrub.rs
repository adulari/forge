//! Defense-in-depth scrubbing for issue reports. Clients scrub before sending; the relay never
//! trusts that. Every free-text field is run through [`scrub`] before it is stored or published,
//! so an outdated or malicious client still cannot put a secret, an e-mail address, a username or
//! an absolute path into a public GitHub issue.

use regex::{Captures, Regex};
use std::sync::LazyLock;

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("static regex")
}

static URL: LazyLock<Regex> =
    LazyLock::new(|| re(r#"(?i)\b([a-z][a-z0-9+.-]*)://([^\s"'<>)\]]+)"#));
static EMAIL: LazyLock<Regex> =
    LazyLock::new(|| re(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+"));
static KEYWORD_VALUE: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r#"(?i)\b((?:api[_-]?key|apikey|token|secret|password|passwd|authorization|auth)["']?\s*[=:]\s*["']?)[^\s"',;]+"#,
    )
});
static BEARER: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)\b(bearer|basic)\s+[A-Za-z0-9._~+/=-]{8,}"));
static SECRET_SHAPES: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"\bsk-[A-Za-z0-9_-]{12,}",
        r"\bgh[pousr]_[A-Za-z0-9]{16,}",
        r"\bgithub_pat_[A-Za-z0-9_]{20,}",
        r"\bAKIA[0-9A-Z]{12,}",
        r"\bAIza[0-9A-Za-z_-]{20,}",
        r"\bxox[abprs]-[A-Za-z0-9-]{10,}",
        r"\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]*",
        r"\b[A-Fa-f0-9]{32,}\b",
    ]
    .iter()
    .map(|p| re(p))
    .collect()
});
static LONG_TOKEN: LazyLock<Regex> = LazyLock::new(|| re(r"[A-Za-z0-9+_=-]{32,}"));
static HOME_DIR: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)(?:/home/|/Users/|[A-Z]:\\Users\\|[A-Z]:/Users/)[^/\\\s:]+"));
static UNIX_PATH: LazyLock<Regex> =
    LazyLock::new(|| re(r#"(?:^|[\s"'(=\[,])((?:~?/[^\s/\\"'<>()\[\],;:]+)+/?)"#));
static WIN_PATH: LazyLock<Regex> = LazyLock::new(|| re(r"[A-Za-z]:\\(?:[^\\\s]+\\)*([^\\\s]*)"));
static WIN_HOME: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^[A-Z]:\\Users\\[^\\]+\\?$"));
static IPV4: LazyLock<Regex> = LazyLock::new(|| re(r"\b\d{1,3}(?:\.\d{1,3}){3}\b"));
static HOST_PORT: LazyLock<Regex> =
    LazyLock::new(|| re(r"\b[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+:\d{2,5}\b"));
static BAD_CONTROL: LazyLock<Regex> = LazyLock::new(|| re(r"[\x00-\x08\x0b-\x1f\x7f]"));

const SOURCE_EXTENSIONS: &[&str] = &[
    "rs", "ts", "tsx", "js", "jsx", "mjs", "py", "go", "c", "h", "cc", "cpp", "kt", "java",
    "swift", "toml", "json", "md", "rb", "m", "mm", "cs",
];

fn looks_like_secret(token: &str) -> bool {
    let has_digit = token.bytes().any(|b| b.is_ascii_digit());
    let has_alpha = token.bytes().any(|b| b.is_ascii_alphabetic());
    has_digit && has_alpha
}

fn host_only(rest: &str) -> &str {
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    authority.rsplit('@').next().unwrap_or("")
}

/// Scrub one free-text field. Idempotent: scrubbing already-scrubbed text changes nothing.
pub(crate) fn scrub(text: &str) -> String {
    let mut out = BAD_CONTROL.replace_all(text, " ").into_owned();
    out = URL
        .replace_all(&out, |c: &Captures| {
            let host = host_only(&c[2]);
            let host = host.split(':').next().unwrap_or(host);
            format!("{}://{host}", &c[1])
        })
        .into_owned();
    out = EMAIL.replace_all(&out, "<email>").into_owned();
    out = BEARER.replace_all(&out, "$1 <redacted>").into_owned();
    out = KEYWORD_VALUE
        .replace_all(&out, "${1}<redacted>")
        .into_owned();
    for shape in SECRET_SHAPES.iter() {
        out = shape.replace_all(&out, "<redacted>").into_owned();
    }
    out = LONG_TOKEN
        .replace_all(&out, |c: &Captures| {
            if looks_like_secret(&c[0]) {
                "<redacted>".to_string()
            } else {
                c[0].to_string()
            }
        })
        .into_owned();
    out = WIN_PATH
        .replace_all(&out, |c: &Captures| {
            if WIN_HOME.is_match(&c[0]) {
                "~".to_string()
            } else {
                c[1].to_string()
            }
        })
        .into_owned();
    out = HOME_DIR.replace_all(&out, "~").into_owned();
    out = UNIX_PATH
        .replace_all(&out, |c: &Captures| {
            let whole = &c[0];
            let path = &c[1];
            let prefix = &whole[..whole.len() - path.len()];
            let base = path.trim_end_matches('/').rsplit('/').next().unwrap_or("");
            if base.is_empty() || base == "~" {
                format!("{prefix}~")
            } else {
                format!("{prefix}{base}")
            }
        })
        .into_owned();
    out = HOST_PORT
        .replace_all(&out, |c: &Captures| {
            let host = c[0].rsplit_once(':').map_or("", |(h, _)| h);
            let ext = host.rsplit('.').next().unwrap_or("");
            if SOURCE_EXTENSIONS.contains(&ext) {
                c[0].to_string()
            } else {
                "<host>".to_string()
            }
        })
        .into_owned();
    out = IPV4.replace_all(&out, "<ip>").into_owned();
    out
}

static QUOTED: LazyLock<Regex> = LazyLock::new(|| re(r#""(?:[^"\\]|\\.)*""#));
static SINGLE_QUOTED: LazyLock<Regex> = LazyLock::new(|| re(r"(^|[\s(\[:=,])'[^'\n]*'"));
static BACKTICKED: LazyLock<Regex> = LazyLock::new(|| re(r"`[^`]*`"));
static BLOB: LazyLock<Regex> = LazyLock::new(|| re(r"\{[^{}]*\}"));

/// Drop quoted strings, backtick spans and `{...}` blobs from an error message: the places where a
/// prompt, a reply, tool arguments or file contents would hide inside error text. Applied to the
/// `message` field only; text a user typed on purpose keeps its quotes.
pub(crate) fn strip_content(text: &str) -> String {
    let mut out = QUOTED.replace_all(text, "<str>").into_owned();
    out = SINGLE_QUOTED.replace_all(&out, "${1}<str>").into_owned();
    out = BACKTICKED.replace_all(&out, "<code>").into_owned();
    for _ in 0..4 {
        let next = BLOB.replace_all(&out, "{..}").into_owned();
        if next == out {
            break;
        }
        out = next;
    }
    out
}

/// Scrub and truncate to at most `max` characters.
pub(crate) fn scrub_to(text: &str, max: usize) -> String {
    let cleaned = scrub(text);
    if cleaned.chars().count() <= max {
        return cleaned;
    }
    let mut cut: String = cleaned.chars().take(max.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_paths_and_absolute_paths_lose_their_directories() {
        let out = scrub("failed to open /home/alice/projects/secret-app/src/main.rs: denied");
        assert!(!out.contains("alice"), "{out}");
        assert!(!out.contains("projects"), "{out}");
        assert!(out.contains("main.rs"), "{out}");
        assert_eq!(scrub("cwd is /home/alice"), "cwd is ~");
        assert_eq!(scrub("at /Users/Bob/x/y.txt"), "at y.txt");
        assert_eq!(scrub(r"C:\Users\carol\code\a.rs"), "a.rs");
        assert_eq!(scrub("see ~/notes/todo.md"), "see todo.md");
    }

    #[test]
    fn relative_source_locations_survive() {
        assert_eq!(
            scrub("forge-core/src/session.rs:412"),
            "forge-core/src/session.rs:412"
        );
        assert_eq!(scrub("a and/or b, 1/2"), "a and/or b, 1/2");
    }

    #[test]
    fn secrets_are_redacted() {
        for secret in [
            "sk-ant-api03-AbCdEfGhIjKlMnOpQrStUv",
            "ghp_abcdefghijklmnopqrstuvwxyz0123456789",
            "AKIAIOSFODNN7EXAMPLE",
            "0123456789abcdef0123456789abcdef0123",
            "Zm9vYmFyMTIzNDU2Nzg5MGFiY2RlZmdoaWprbG1ub3A",
        ] {
            let out = scrub(&format!("boom with {secret} inside"));
            assert!(!out.contains(secret), "{secret} leaked: {out}");
        }
        let out = scrub("Authorization: Bearer abcDEF123456xyz");
        assert!(!out.contains("abcDEF123456xyz"), "{out}");
        let out = scrub("api_key=hunter2hunter2 and password: swordfish");
        assert!(
            !out.contains("hunter2") && !out.contains("swordfish"),
            "{out}"
        );
    }

    #[test]
    fn urls_keep_only_the_host_and_emails_vanish() {
        let out =
            scrub("GET https://user:pw@api.example.com:8443/v1/x?key=1#f failed for bob@corp.io");
        assert!(out.contains("https://api.example.com"), "{out}");
        assert!(
            !out.contains("/v1") && !out.contains("pw@") && !out.contains("bob"),
            "{out}"
        );
        assert!(out.contains("<email>"));
    }

    #[test]
    fn bare_hosts_with_ports_and_ips_are_masked() {
        let out = scrub("connect db.internal.corp:5432 via 10.1.2.3");
        assert!(
            !out.contains("internal") && !out.contains("10.1.2.3"),
            "{out}"
        );
    }

    #[test]
    fn scrub_is_idempotent_and_truncates() {
        let once = scrub("/home/a/b/c.rs token=abcdef123456 https://h.io/p");
        assert_eq!(scrub(&once), once);
        assert_eq!(scrub_to(&"x".repeat(50), 10).chars().count(), 10);
    }

    #[test]
    fn messages_lose_quoted_content_and_blobs() {
        let out = strip_content(
            r#"Err(Parse("my prompt about the merger")) with `secret_fn()` and {"cmd": "rm -rf /"} but can't stop"#,
        );
        assert!(!out.contains("merger") && !out.contains("secret_fn") && !out.contains("rm -rf"));
        assert!(out.contains("can't stop"), "{out}");
    }

    #[test]
    fn long_identifiers_without_digits_are_kept() {
        let ident = "forge_core_session_run_turn_inner_helper_function_name";
        assert_eq!(scrub(ident), ident);
    }
}
