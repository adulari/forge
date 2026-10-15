//! The throwaway repo the recall scenario works in: a tiny header-parser crate plus bulk data
//! files whose only job is to make the model read enough text for the context to grow.
//! Nothing in it states a planted fact (port, rule, file name), so an answer cannot be read
//! back off disk, except the ones the scenario itself edits in.

use std::path::Path;

use anyhow::{Context, Result};

const CARGO_TOML: &str =
    "[package]\nname = \"hdrparse\"\nversion = \"0.4.1\"\nedition = \"2021\"\n\n[dependencies]\n";

const README: &str = "# hdrparse\n\nA small HTTP/1.1 header parser. The lexer splits a header line into tokens, the parser turns\ntokens into a `Header`, and the server module wires both into a request handler.\n";

const MAIN_RS: &str = "mod config;\nmod lexer;\nmod limits;\nmod parser;\nmod server;\nmod tables;\n\nfn main() {\n    let cfg = config::Config::default();\n    server::serve(&cfg);\n}\n";

const PARSER_RS: &str = r#"use crate::lexer::{tok_next, Token};

#[derive(Debug, PartialEq)]
pub enum ParseError {
    Empty,
    BadName,
    BadValue,
}

#[derive(Debug, PartialEq)]
pub struct Header {
    pub name: String,
    pub value: String,
}

pub fn parse_header(line: &str) -> Result<Header, ParseError> {
    if line.trim().is_empty() {
        return Err(ParseError::Empty);
    }
    let mut pos = 0;
    let name = match tok_next(line, &mut pos) {
        Some(Token::Word(w)) => w,
        _ => return Err(ParseError::BadName),
    };
    match tok_next(line, &mut pos) {
        Some(Token::Colon) => {}
        _ => return Err(ParseError::BadName),
    }
    let value = line[pos..].trim().to_string();
    if value.is_empty() {
        return Err(ParseError::BadValue);
    }
    Ok(Header { name, value })
}
"#;

const LEXER_RS: &str = r#"#[derive(Debug, PartialEq)]
pub enum Token {
    Word(String),
    Colon,
    Space,
}

pub fn tok_next(input: &str, pos: &mut usize) -> Option<Token> {
    let bytes = input.as_bytes();
    if *pos >= bytes.len() {
        return None;
    }
    match bytes[*pos] {
        b':' => {
            *pos += 1;
            Some(Token::Colon)
        }
        b' ' | b'\t' => {
            while *pos < bytes.len() && (bytes[*pos] == b' ' || bytes[*pos] == b'\t') {
                *pos += 1;
            }
            Some(Token::Space)
        }
        _ => {
            let start = *pos;
            while *pos < bytes.len() && bytes[*pos] != b':' && bytes[*pos] != b' ' {
                *pos += 1;
            }
            Some(Token::Word(input[start..*pos].to_string()))
        }
    }
}

pub fn count_tokens(input: &str) -> usize {
    let mut pos = 0;
    let mut n = 0;
    while tok_next(input, &mut pos).is_some() {
        n += 1;
    }
    n
}
"#;

const LIMITS_RS: &str = "use crate::parser::ParseError;\n\n/// Longest header section we accept, in bytes.\npub const MAX_HEADER_BYTES: usize = 8192;\n\npub const MAX_HEADERS: usize = 64;\n";

const CONFIG_RS: &str = "pub struct Config {\n    pub bind: String,\n    pub timeout_secs: u32,\n}\n\nimpl Default for Config {\n    fn default() -> Self {\n        Self {\n            bind: \"127.0.0.1:8080\".into(),\n            timeout_secs: 30,\n        }\n    }\n}\n\npub fn describe(cfg: &Config) -> String {\n    format!(\"{} (timeout {}s)\", cfg.bind, cfg.timeout_secs)\n}\n";

const SERVER_RS: &str = "use crate::config::Config;\nuse crate::parser::{parse_header, Header};\n\n// TODO: stream large bodies instead of buffering them\npub fn handle_request(raw: &str) -> Vec<Header> {\n    raw.lines().filter_map(|l| parse_header(l).ok()).collect()\n}\n\npub fn serve(cfg: &Config) {\n    // TODO: actually bind a socket\n    let _ = crate::config::describe(cfg);\n}\n";

const CHECK_SH: &str = "#!/usr/bin/env bash\nif ! grep -q 'MAX_HEADER_BYTES: usize = 16384' src/limits.rs; then\n  echo \"ERROR E-4172: header length limit too small: MAX_HEADER_BYTES (8192) rejects valid 12 KiB cookies\" >&2\n  exit 1\nfi\necho \"check ok\"\n";

fn status_codes() -> String {
    (0..300)
        .map(|i| {
            format!(
                "{i:04},STATUS_{i:04},retryable={},desc=\"entry number {i} of the generated status table\"\n",
                i % 3 == 0
            )
        })
        .collect()
}

fn notes() -> String {
    let mut out = String::from("# Design notes\n\n");
    for i in 1..=40 {
        out.push_str(&format!(
            "## Note {i}\n\nThis note records background on topic {i}: how the module boundary was \
drawn, which alternatives were considered, and what the benchmark showed at the time. It is \
filler text for the recall benchmark and carries no instructions.\n\n"
        ));
    }
    out
}

fn tables() -> String {
    (0..160)
        .map(|i| format!("pub const TABLE_ENTRY_{i:03}: u16 = {};\n", (i * 37) % 1009))
        .collect()
}

fn files() -> Vec<(&'static str, String)> {
    vec![
        ("Cargo.toml", CARGO_TOML.into()),
        ("README.md", README.into()),
        (".gitignore", "target/\n.forge/\n".into()),
        ("src/main.rs", MAIN_RS.into()),
        ("src/parser.rs", PARSER_RS.into()),
        ("src/lexer.rs", LEXER_RS.into()),
        ("src/limits.rs", LIMITS_RS.into()),
        ("src/config.rs", CONFIG_RS.into()),
        ("src/server.rs", SERVER_RS.into()),
        ("src/tables.rs", tables()),
        ("scripts/check.sh", CHECK_SH.into()),
        ("data/status_codes.txt", status_codes()),
        ("docs/NOTES.md", notes()),
    ]
}

fn git(dir: &Path, args: &[&str]) -> Result<()> {
    crate::bench::run_git(dir, args).map(|_| ())
}

/// Write the fixture into `dir` and make it a git repo with one baseline commit, so the
/// "commit your changes" probe has something to commit onto.
pub(super) fn create(dir: &Path) -> Result<()> {
    for (rel, body) in files() {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        std::fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
    }
    git(dir, &["init", "-q"])?;
    git(dir, &["config", "user.email", "bench@forge.invalid"])?;
    git(dir, &["config", "user.name", "forge-bench"])?;
    git(dir, &["config", "commit.gpgsign", "false"])?;
    git(dir, &["add", "-A"])?;
    git(dir, &["commit", "-q", "-m", "baseline"])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fixture_builds_a_committed_repo_without_stating_any_planted_fact() {
        let tmp = tempfile::tempdir().unwrap();
        create(tmp.path()).unwrap();
        assert!(tmp.path().join("src/parser.rs").is_file());
        let log = crate::bench::run_git(tmp.path(), &["log", "--format=%s"]).unwrap();
        assert_eq!(log.trim(), "baseline");
        for (rel, body) in files() {
            for planted in [
                "47123",
                "settings.toml",
                "JIRA-123",
                "validate_header_len",
                "next_token",
            ] {
                assert!(!body.contains(planted), "{rel} must not contain {planted}");
            }
        }
        assert!(
            !PARSER_RS.contains("unwrap"),
            "the baseline parser obeys its own rule"
        );
        assert!(status_codes().contains("0042,STATUS_0042"));
    }
}
