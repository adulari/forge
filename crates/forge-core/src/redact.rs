//! Secret redaction for everything Forge writes out of its store: `forge replay` (transcript,
//! `--json`, `--html`), the TUI `/export`, and `forge anywhere share` (docs/features/redaction.md).
//!
//! Three layers, applied in order:
//! 1. **Known secrets** — every configured provider/search key (env, keyring, encrypted fallback)
//!    plus the values of env vars named like `*_KEY` / `*_TOKEN` / `*_SECRET` / `*_PASSWORD`.
//!    Exact-match replacement; this is the only layer that is a guarantee.
//! 2. **Shapes** — well-known credential formats (`sk-…`, `ghp_…`, `AKIA…`, `xox?-…`, JWTs,
//!    private-key PEM blocks, `Bearer …`, `scheme://user:pass@`).
//! 3. **Keyword-adjacent values** — `password = …`, `"api_key": "…"`, `TOKEN=…`.
//!
//! Every pattern is anchored on a prefix or a keyword, so bare git SHAs and UUIDs never match.

use std::borrow::Cow;
use std::sync::LazyLock;

use forge_store::ReplayEntry;
use regex::{Captures, Regex};

pub const REDACTED: &str = "[redacted]";

/// Shorter "secrets" (a 4-char PIN in an env var, `KEYMAP=us`) would turn ordinary words in a
/// transcript into `[redacted]`; no real credential is this short.
const MIN_KNOWN_SECRET_LEN: usize = 8;

/// Search/aux keys stored in the keyring under their own names (not model providers).
const EXTRA_SECRET_KEYS: &[&str] = &["artificialanalysis"];

struct Shape {
    re: Regex,
    replacement: &'static str,
}

fn shape(pattern: &str, replacement: &'static str) -> Shape {
    Shape {
        re: Regex::new(pattern).expect("redaction shape regex compiles"),
        replacement,
    }
}

static SHAPES: LazyLock<Vec<Shape>> = LazyLock::new(|| {
    vec![
        shape(
            r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z0-9 ]*PRIVATE KEY-----",
            REDACTED,
        ),
        // OpenAI (`sk-`, `sk-proj-`) and Anthropic (`sk-ant-api…`, `sk-ant-oat…`).
        shape(r"\bsk-[A-Za-z0-9_\-]{20,}", REDACTED),
        shape(
            r"\b(?:gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{30,})",
            REDACTED,
        ),
        shape(r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b", REDACTED),
        shape(r"\bxox[abposr]-[A-Za-z0-9\-]{10,}", REDACTED),
        shape(r"\bAIza[0-9A-Za-z_\-]{35}", REDACTED),
        shape(r"\bhf_[A-Za-z0-9]{30,}", REDACTED),
        shape(
            r"\beyJ[A-Za-z0-9_\-]{10,}\.eyJ[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]*",
            REDACTED,
        ),
        // 16+ chars so prose like "a Bearer token" survives.
        shape(
            r"(?P<keep>(?i:\bbearer)\s+)[A-Za-z0-9._~+/\-]{16,}=*",
            "${keep}[redacted]",
        ),
        shape(
            r"(?P<keep>\b[a-z][a-z0-9+.\-]*://[^/\s:@]+:)[^/\s@]+@",
            "${keep}[redacted]@",
        ),
    ]
});

static KEYWORD_VALUE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?P<keep>\b(?:[A-Za-z0-9]+[_\-])*(?i:password|passwd|secret|token|api[_\-]?key|access[_\-]?key|private[_\-]?key|client[_\-]?secret)\b(?:\\?["'])?\s*[:=]\s*(?:\\?["'])?)(?P<val>[^\s"'\\,;(){}\[\]<>]{6,})"#,
    )
    .expect("keyword redaction regex compiles")
});

/// `token: usize`, `password = getpass`, `secret: self.secret`, `api_key=${API_KEY}`: code and
/// placeholders, not values. Real secrets carry a digit or a symbol, and every common credential
/// format is caught by a shape anyway.
fn is_code_or_placeholder(value: &str) -> bool {
    let identifier_like = value
        .chars()
        .all(|c| c.is_ascii_alphabetic() || c == '_' || c == '.');
    let numeric = value.chars().all(|c| c.is_ascii_digit());
    identifier_like || numeric || value.starts_with('$') || value.starts_with('%')
}

#[derive(Debug, Clone, Default)]
pub struct Redactor {
    /// Longest first, so a secret that contains another is replaced whole.
    known: Vec<String>,
}

impl Redactor {
    pub fn new(known: impl IntoIterator<Item = String>) -> Self {
        let mut known: Vec<String> = known
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| s.chars().count() >= MIN_KNOWN_SECRET_LEN)
            .collect();
        known.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
        known.dedup();
        Redactor { known }
    }

    /// A redactor seeded with every secret this machine has configured.
    pub fn from_environment() -> Self {
        Self::new(known_secrets())
    }

    pub fn known_count(&self) -> usize {
        self.known.len()
    }

    pub fn redact<'a>(&self, text: &'a str) -> Cow<'a, str> {
        let mut out = Cow::Borrowed(text);
        for secret in &self.known {
            if out.contains(secret.as_str()) {
                out = Cow::Owned(out.replace(secret.as_str(), REDACTED));
            }
        }
        for s in SHAPES.iter() {
            if let Cow::Owned(replaced) = s.re.replace_all(&out, s.replacement) {
                out = Cow::Owned(replaced);
            }
        }
        let keyword = KEYWORD_VALUE.replace_all(&out, |caps: &Captures| {
            let value = &caps["val"];
            if is_code_or_placeholder(value) {
                caps[0].to_string()
            } else {
                format!("{}{REDACTED}", &caps["keep"])
            }
        });
        if let Cow::Owned(replaced) = keyword {
            if replaced != *out {
                out = Cow::Owned(replaced);
            }
        }
        out
    }

    /// Redact every string inside a JSON value in place (tool-call arguments).
    pub fn redact_json(&self, value: &mut serde_json::Value) {
        match value {
            serde_json::Value::String(s) => {
                if let Cow::Owned(r) = self.redact(s) {
                    *s = r;
                }
            }
            serde_json::Value::Array(items) => items.iter_mut().for_each(|v| self.redact_json(v)),
            serde_json::Value::Object(map) => map.values_mut().for_each(|v| self.redact_json(v)),
            _ => {}
        }
    }

    /// Redact message content and tool-call arguments of a replay before it is rendered.
    pub fn redact_replay(&self, entries: &mut [ReplayEntry]) {
        for entry in entries {
            if let Cow::Owned(r) = self.redact(&entry.content) {
                entry.content = r;
            }
            for call in &mut entry.tool_calls {
                self.redact_json(&mut call.args);
            }
        }
    }
}

/// One-shot form of [`Redactor::redact`].
pub fn redact<'a>(text: &'a str, known: &[String]) -> Cow<'a, str> {
    match Redactor::new(known.iter().cloned()).redact(text) {
        Cow::Borrowed(_) => Cow::Borrowed(text),
        Cow::Owned(s) => Cow::Owned(s),
    }
}

/// Whether an env var NAME marks its value as a credential: an `_`-separated segment is one of
/// KEY/PAT/… or ends in TOKEN/SECRET/PASSWORD/… (`OPENAI_API_KEY`, `PGPASSWORD`), so `GNOME_KEYRING_CONTROL`
/// or `TOKENIZERS_PARALLELISM` do not qualify.
pub fn is_secret_env_name(name: &str) -> bool {
    name.to_ascii_uppercase().split(['_', '-']).any(|seg| {
        matches!(seg, "KEY" | "PASS" | "PAT" | "CREDENTIAL" | "CREDENTIALS")
            || ["APIKEY", "TOKEN", "SECRET", "PASSWORD", "PASSWD"]
                .iter()
                .any(|suffix| seg.ends_with(suffix))
    })
}

/// Env values that cannot be credentials even under a secret-sounding name: paths, booleans,
/// sentences.
fn plausible_secret_value(value: &str) -> bool {
    let v = value.trim();
    !(v.starts_with('/')
        || v.starts_with('~')
        || v.contains(char::is_whitespace)
        || matches!(
            v.to_ascii_lowercase().as_str(),
            "true" | "false" | "enabled" | "disabled"
        ))
}

fn split_keys(raw: &str) -> impl Iterator<Item = String> + '_ {
    raw.split([',', '\n'])
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .map(str::to_string)
}

/// Every secret value configured on this machine: provider and search keys from env + keyring +
/// encrypted fallback, and secret-named env vars.
pub fn known_secrets() -> Vec<String> {
    let providers: Vec<&str> = forge_config::known_key_providers()
        .chain(forge_config::known_search_providers())
        .chain(EXTRA_SECRET_KEYS.iter().copied())
        .collect();
    // Each keyring lookup is a D-Bus round trip; warm the memoized cache in parallel first, the
    // same way `inject_provider_keys` does.
    std::thread::scope(|scope| {
        let per_thread = providers.len().div_ceil(8).max(1);
        for chunk in providers.chunks(per_thread) {
            scope.spawn(move || {
                for p in chunk {
                    let _ = forge_config::secret_store::get(p);
                }
            });
        }
    });
    let mut out: Vec<String> = Vec::new();
    for p in &providers {
        out.extend(forge_config::api_keys(p));
        if let Some(stored) = forge_config::secret_store::get(p) {
            out.extend(split_keys(&stored));
        }
    }
    for (name, value) in std::env::vars() {
        if is_secret_env_name(&name) && plausible_secret_value(&value) {
            out.extend(split_keys(&value));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(text: &str) -> String {
        Redactor::default().redact(text).into_owned()
    }

    #[test]
    fn known_secrets_are_replaced_exactly_longest_first() {
        let red = Redactor::new(["abcd1234efgh".to_string(), "abcd1234efgh5678".to_string()]);
        assert_eq!(
            red.redact("k=abcd1234efgh5678 and abcd1234efgh"),
            "k=[redacted] and [redacted]"
        );
        assert_eq!(red.known_count(), 2);
    }

    #[test]
    fn short_known_values_are_ignored() {
        let red = Redactor::new(["us".to_string(), "1234".to_string()]);
        assert_eq!(red.known_count(), 0);
        assert_eq!(red.redact("focus on 1234 users"), "focus on 1234 users");
    }

    #[test]
    fn untouched_text_stays_borrowed() {
        let red = Redactor::new(["supersecretvalue".to_string()]);
        assert!(matches!(red.redact("nothing here"), Cow::Borrowed(_)));
    }

    #[test]
    fn every_shape_is_masked() {
        let cases = [
            "sk-proj-AbCdEfGhIjKlMnOpQrStUvWx12",
            "sk-ant-api03-AbCdEfGhIjKlMnOpQrStUvWxYz0123",
            "sk-ant-oat01-AbCdEfGhIjKlMnOpQrStUvWxYz0123",
            "ghp_AbCdEfGhIjKlMnOpQrStUvWxYz0123456789",
            "gho_AbCdEfGhIjKlMnOpQrStUvWxYz0123456789",
            "github_pat_11ABCDEFG0123456789_abcdefghijklmnopqrstuv",
            "AKIAIOSFODNN7EXAMPLE",
            "xoxb-123456789012-abcdefghij",
            "AIzaSyA1234567890abcdefghijklmnopqrstuv",
            "hf_AbCdEfGhIjKlMnOpQrStUvWxYz012345",
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U",
        ];
        for secret in cases {
            let out = r(&format!("value {secret} end"));
            assert_eq!(out, "value [redacted] end", "{secret} must be masked");
        }
    }

    #[test]
    fn pem_private_key_block_is_masked_but_certificates_survive() {
        let pem = "-----BEGIN RSA PRIVATE KEY-----\nMIIEow\nabc\n-----END RSA PRIVATE KEY-----";
        assert_eq!(r(&format!("key:\n{pem}\n")), "key:\n[redacted]\n");
        let cert = "-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----";
        assert_eq!(r(cert), cert);
    }

    #[test]
    fn bearer_and_url_credentials_keep_their_prefix() {
        assert_eq!(
            r("Authorization: Bearer abcdef0123456789abcdef"),
            "Authorization: Bearer [redacted]"
        );
        assert_eq!(r("send a Bearer token"), "send a Bearer token");
        assert_eq!(
            r("git clone https://bob:hunter2pass@github.com/x/y"),
            "git clone https://bob:[redacted]@github.com/x/y"
        );
    }

    #[test]
    fn keyword_adjacent_values_are_masked() {
        assert_eq!(r("password=hunter22"), "password=[redacted]");
        assert_eq!(
            r("OPENAI_API_KEY=abc123xyz789"),
            "OPENAI_API_KEY=[redacted]"
        );
        assert_eq!(
            r(r#"{"api_key": "q8w7e6r5t4"}"#),
            r#"{"api_key": "[redacted]"}"#
        );
        assert_eq!(
            r(r#"{\"client_secret\":\"x9y8z7w6\"}"#),
            r#"{\"client_secret\":\"[redacted]\"}"#
        );
        assert_eq!(r("db secret: s3cr3t-v4lue"), "db secret: [redacted]");
    }

    #[test]
    fn keyword_next_to_code_is_left_alone() {
        for code in [
            "let token: usize = 3;",
            "password = getpass()",
            "secret: self.secret",
            "api_key=${API_KEY}",
            "max_tokens=100000",
            "token: Option<String>",
            "input_token: 123456",
        ] {
            assert_eq!(r(code), code, "{code} must survive");
        }
    }

    #[test]
    fn git_shas_and_uuids_survive() {
        let text = "commit 3f8c377b9a40bbb0551e2dde96f895b75267da63 short 3f8c377b \
                    session 0124vfZs uuid 550e8400-e29b-41d4-a716-446655440000 \
                    sha256:9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08 \
                    task-AbCdEfGhIjKlMnOpQrStUvWx";
        assert_eq!(r(text), text);
    }

    #[test]
    fn redacted_marker_is_not_redacted_again() {
        let once = r("token=abc123def456");
        assert_eq!(r(&once), once);
    }

    #[test]
    fn json_and_replay_entries_are_redacted_in_place() {
        let red = Redactor::new(["plantedSecretValue42".to_string()]);
        let mut entries = vec![ReplayEntry {
            seq: 1,
            role: forge_types::Role::Assistant,
            content: "using plantedSecretValue42".into(),
            model: None,
            created_at: 0,
            tool_calls: vec![forge_types::ToolCall {
                id: "c1".into(),
                name: "shell".into(),
                args: serde_json::json!({
                    "command": "curl -H 'x-api-key: plantedSecretValue42' https://x",
                    "nested": ["plantedSecretValue42", 3],
                }),
            }],
            input_tokens: None,
            output_tokens: None,
            cost_usd: None,
        }];
        red.redact_replay(&mut entries);
        let dumped = serde_json::to_string(&entries[0].tool_calls[0].args).unwrap();
        assert!(!dumped.contains("plantedSecretValue42"), "{dumped}");
        assert!(!entries[0].content.contains("plantedSecretValue42"));
        assert_eq!(entries[0].tool_calls[0].args["nested"][1], 3);
    }

    #[test]
    fn secret_env_names_match_by_segment() {
        for yes in [
            "OPENAI_API_KEY",
            "GITHUB_TOKEN",
            "AWS_SECRET_ACCESS_KEY",
            "PGPASSWORD_X",
            "HF_TOKEN",
            "my-key",
        ] {
            assert!(is_secret_env_name(yes), "{yes}");
        }
        for no in [
            "GNOME_KEYRING_CONTROL",
            "TOKENIZERS_PARALLELISM",
            "PATH",
            "XKB_KEYMAP",
        ] {
            assert!(!is_secret_env_name(no), "{no}");
        }
        assert!(!plausible_secret_value("/run/user/1000/keyring"));
        assert!(!plausible_secret_value("false"));
        assert!(plausible_secret_value("abc123def456"));
    }

    #[test]
    fn known_secrets_include_secret_named_env_vars() {
        let name = "FORGE_REDACT_TEST_TOKEN";
        std::env::set_var(name, "envPlantedValue9876");
        let secrets = known_secrets();
        std::env::remove_var(name);
        assert!(secrets.iter().any(|s| s == "envPlantedValue9876"));
    }

    #[test]
    fn one_shot_helper_matches_the_redactor() {
        let known = vec!["oneShotSecret123".to_string()];
        assert_eq!(redact("x oneShotSecret123", &known), "x [redacted]");
        assert!(matches!(redact("plain", &known), Cow::Borrowed(_)));
    }
}
