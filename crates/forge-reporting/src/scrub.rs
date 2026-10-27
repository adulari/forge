//! Everything that leaves the machine passes through here first.
//!
//! The rules are deliberately blunt: a report is a bug description, never a transcript, so any text
//! that could carry user data is reduced to its shape. Secrets are redacted (the host registers the
//! full `forge-core` redactor via [`set_redactor`]; the built-in shapes below are the floor), URLs
//! keep only their host, the home directory becomes `~`, every other absolute path keeps only its
//! basename, and quoted strings, backtick spans and `{...}` blobs (where prompts, replies, tool
//! arguments and file contents show up inside error text) are dropped from messages.

use std::sync::{Arc, OnceLock, RwLock};

use regex::{Captures, Regex};

type Redactor = Arc<dyn Fn(&str) -> String + Send + Sync>;

static REDACTOR: RwLock<Option<Redactor>> = RwLock::new(None);

/// Register the host's secret redactor (`forge_core::redact::Redactor::from_environment`). It runs
/// before the built-in shapes, so known provider keys are removed even when they carry no
/// recognisable prefix.
pub fn set_redactor(redactor: impl Fn(&str) -> String + Send + Sync + 'static) {
    if let Ok(mut slot) = REDACTOR.write() {
        *slot = Some(Arc::new(redactor));
    }
}

/// Who and where this process is, so their names can be removed from any text.
#[derive(Debug, Clone, Default)]
pub struct Identity {
    pub home: Option<String>,
    pub user: Option<String>,
    pub host: Option<String>,
}

impl Identity {
    pub fn detect() -> Self {
        let home = forge_config::home_dir()
            .map(|p| {
                p.to_string_lossy()
                    .trim_end_matches(['/', '\\'])
                    .to_string()
            })
            .filter(|h| h.len() > 1);
        let user = ["USER", "USERNAME", "LOGNAME"]
            .iter()
            .find_map(|k| std::env::var(k).ok())
            .map(|u| u.trim().to_string())
            .filter(|u| u.len() >= 3);
        let host = hostname().filter(|h| h.len() >= 3);
        Identity { home, user, host }
    }
}

fn hostname() -> Option<String> {
    #[cfg(unix)]
    {
        let mut buf = [0u8; 256];
        // SAFETY: the buffer is valid for `buf.len()` bytes and gethostname NUL-terminates it on
        // success.
        let rc = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
        if rc == 0 {
            let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
            let name = String::from_utf8_lossy(&buf[..end]).trim().to_string();
            if !name.is_empty() {
                return Some(name);
            }
        }
    }
    ["HOSTNAME", "COMPUTERNAME"]
        .iter()
        .find_map(|k| std::env::var(k).ok())
        .map(|h| h.trim().to_string())
}

fn identity() -> &'static Identity {
    static ID: OnceLock<Identity> = OnceLock::new();
    ID.get_or_init(Identity::detect)
}

struct Shapes {
    secrets: Vec<(Regex, &'static str)>,
    keyword: Regex,
    long_token: Regex,
    email: Regex,
    url: Regex,
    ipv4: Regex,
    unix_path: Regex,
    windows_path: Regex,
    dq: Regex,
    sq: Regex,
    backtick: Regex,
    blob: Regex,
}

fn shapes() -> &'static Shapes {
    static SHAPES: OnceLock<Shapes> = OnceLock::new();
    SHAPES.get_or_init(|| {
        let re = |p: &str| Regex::new(p).expect("static scrub regex");
        Shapes {
            secrets: vec![
                (re(r"(?i)\bbearer\s+[A-Za-z0-9._~+/=-]{8,}"), "Bearer <redacted>"),
                (re(r"\bsk-[A-Za-z0-9_-]{10,}"), "<redacted-key>"),
                (re(r"\bsk_[a-z]+_[A-Za-z0-9]{10,}"), "<redacted-key>"),
                (re(r"\bgh[pousr]_[A-Za-z0-9]{16,}"), "<redacted-key>"),
                (re(r"\bgithub_pat_[A-Za-z0-9_]{20,}"), "<redacted-key>"),
                (re(r"\bAKIA[0-9A-Z]{16}\b"), "<redacted-key>"),
                (re(r"\bAIza[0-9A-Za-z_-]{30,}"), "<redacted-key>"),
                (re(r"\bxox[abprs]-[A-Za-z0-9-]{10,}"), "<redacted-key>"),
                (re(r"\bnvapi-[A-Za-z0-9_-]{10,}"), "<redacted-key>"),
                (
                    re(r"\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]*"),
                    "<redacted-jwt>",
                ),
                (
                    re(r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?(?:-----END [A-Z ]*PRIVATE KEY-----|$)"),
                    "<redacted-private-key>",
                ),
            ],
            keyword: re(
                r#"(?i)\b(api[_-]?key|access[_-]?token|auth[_-]?token|token|secret|password|passwd|authorization|credential)s?\b(\s*[:=]\s*)("[^"]*"|'[^']*'|[^\s,;&]+)"#,
            ),
            long_token: re(r"[A-Za-z0-9_\-+/=]{32,}"),
            email: re(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+"),
            url: re(
                r#"(?i)\b([a-z][a-z0-9+.-]*)://(?:[^\s/@"'<>]*@)?([^\s/:?#"'<>)\]]+)(?::\d+)?(?:[/?#][^\s"'<>)\]]*)?"#,
            ),
            ipv4: re(r"\b(?:\d{1,3}\.){3}\d{1,3}\b"),
            unix_path: re(
                r#"(?P<pre>^|[\s"'(\[=,;<>{])(?P<path>/[^\s"'()\[\]<>{}|,;]+)"#,
            ),
            windows_path: re(r#"[A-Za-z]:\\(?:[^\s"'<>|\\]+\\)*(?P<base>[^\s"'<>|\\]+)"#),
            dq: re(r#""(?:[^"\\]|\\.)*""#),
            sq: re(r"(?P<pre>^|[\s(\[:=,])'[^'\n]*'"),
            backtick: re(r"`[^`]*`"),
            blob: re(r"\{[^{}]*\}"),
        }
    })
}

fn is_internal_host(host: &str, id: &Identity) -> bool {
    let h = host.to_ascii_lowercase();
    if h == "localhost" || !h.contains('.') {
        return true;
    }
    if h.parse::<std::net::IpAddr>().is_ok() {
        return true;
    }
    if [
        ".local",
        ".internal",
        ".lan",
        ".home",
        ".corp",
        ".intranet",
        ".localdomain",
        ".ts.net",
    ]
    .iter()
    .any(|s| h.ends_with(s))
    {
        return true;
    }
    id.host
        .as_ref()
        .is_some_and(|own| h.contains(&own.to_ascii_lowercase()))
}

fn basename(path: &str) -> &str {
    let trimmed = path.trim_end_matches(['/', '.', ':']);
    trimmed
        .rsplit('/')
        .next()
        .filter(|b| !b.is_empty())
        .unwrap_or("<path>")
}

/// Replace `needle` as a whole word, case-insensitively.
fn replace_word(text: &str, needle: &str, with: &str) -> String {
    if needle.is_empty() {
        return text.to_string();
    }
    let pattern = format!(r"(?i)\b{}\b", regex::escape(needle));
    match Regex::new(&pattern) {
        Ok(re) => re.replace_all(text, with).into_owned(),
        Err(_) => text.to_string(),
    }
}

/// Scrub free text against an explicit identity. [`scrub`] uses the process's own.
pub fn scrub_with(text: &str, id: &Identity) -> String {
    let s = shapes();
    let mut out = match REDACTOR.read().ok().and_then(|slot| slot.clone()) {
        Some(redactor) => redactor(text),
        None => text.to_string(),
    };

    for (re, with) in &s.secrets {
        out = re.replace_all(&out, *with).into_owned();
    }
    out = s
        .keyword
        .replace_all(&out, |c: &Captures| {
            let value = c[3].trim_matches(['"', '\'']);
            if value.is_empty() || value.starts_with('<') || value.starts_with('$') {
                c[0].to_string()
            } else {
                format!("{}{}<redacted>", &c[1], &c[2])
            }
        })
        .into_owned();
    out = s
        .long_token
        .replace_all(&out, |c: &Captures| {
            let t = &c[0];
            let has_digit = t.bytes().any(|b| b.is_ascii_digit());
            let has_alpha = t.bytes().any(|b| b.is_ascii_alphabetic());
            if has_digit && has_alpha && !t.contains('/') {
                "<redacted-token>".to_string()
            } else {
                t.to_string()
            }
        })
        .into_owned();

    out = s
        .url
        .replace_all(&out, |c: &Captures| {
            let scheme = c[1].to_ascii_lowercase();
            if scheme == "file" {
                return "<file-url>".to_string();
            }
            let host = &c[2];
            if is_internal_host(host, id) {
                format!("{scheme}://<internal-host>")
            } else {
                format!("{scheme}://{}", host.to_ascii_lowercase())
            }
        })
        .into_owned();
    out = s.email.replace_all(&out, "<email>").into_owned();
    out = s.ipv4.replace_all(&out, "<ip>").into_owned();

    if let Some(home) = &id.home {
        let escaped = regex::escape(home);
        if let Ok(re) = Regex::new(&format!(r"(?i){escaped}(?:[/\\]|\b)")) {
            out = re
                .replace_all(&out, |c: &Captures| {
                    if c[0].ends_with(['/', '\\']) {
                        "~/".to_string()
                    } else {
                        "~".to_string()
                    }
                })
                .into_owned();
        }
    }
    out = s
        .windows_path
        .replace_all(&out, |c: &Captures| c["base"].to_string())
        .into_owned();
    out = s
        .unix_path
        .replace_all(&out, |c: &Captures| {
            format!("{}{}", &c["pre"], basename(&c["path"]))
        })
        .into_owned();

    if let Some(user) = &id.user {
        out = replace_word(&out, user, "<user>");
    }
    if let Some(host) = &id.host {
        out = replace_word(&out, host, "<host>");
    }
    out
}

pub fn scrub(text: &str) -> String {
    scrub_with(text, identity())
}

/// Drop quoted strings, backtick spans and `{...}` blobs: the places where prompts, model replies,
/// tool arguments and file contents hide inside error text.
pub fn strip_content(text: &str) -> String {
    let s = shapes();
    let mut out = s.dq.replace_all(text, "<str>").into_owned();
    out = s.sq.replace_all(&out, "${pre}<str>").into_owned();
    out = s.backtick.replace_all(&out, "<code>").into_owned();
    for _ in 0..4 {
        let next = s.blob.replace_all(&out, "{..}").into_owned();
        if next == out {
            break;
        }
        out = next;
    }
    out
}

/// Cut to `max` characters on a char boundary, marking the cut.
pub fn truncate(text: &str, max: usize) -> String {
    let flat: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let flat = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        return flat;
    }
    let cut: String = flat.chars().take(max.saturating_sub(1)).collect();
    format!("{cut}…")
}

/// An error message reduced to something safe to send: scrubbed, content removed, truncated.
pub fn scrub_message(text: &str, max: usize) -> String {
    truncate(&strip_content(&scrub(text)), max)
}

/// Scrub with an explicit identity (tests), message flavour.
#[cfg(test)]
pub fn scrub_message_with(text: &str, id: &Identity, max: usize) -> String {
    truncate(&strip_content(&scrub_with(text, id)), max)
}

/// Normalise a source location (`/home/u/proj/crates/forge-core/src/a.rs:12:5`) to its in-repo
/// form (`forge-core/src/a.rs:12`); anything else keeps its last three components.
pub fn normalize_location(raw: &str) -> String {
    static LOC: OnceLock<Regex> = OnceLock::new();
    let loc = LOC.get_or_init(|| {
        Regex::new(r"^(?P<path>.*?):(?P<line>\d+)(?::\d+)?$").expect("static regex")
    });
    let raw = raw.trim().trim_start_matches("at ").replace('\\', "/");
    let (path, line) = match loc.captures(&raw) {
        Some(c) => (c["path"].to_string(), c["line"].to_string()),
        None => (raw.clone(), String::new()),
    };
    let short = match path.find("crates/") {
        Some(i) => path[i + "crates/".len()..].to_string(),
        None => {
            let comps: Vec<&str> = path.split('/').filter(|c| !c.is_empty()).collect();
            comps[comps.len().saturating_sub(3)..].join("/")
        }
    };
    if line.is_empty() {
        short
    } else {
        format!("{short}:{line}")
    }
}

#[cfg(test)]
mod tests;
