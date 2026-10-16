//! The throwaway repo the recall scenario works in: a tiny header-parser crate plus bulk data
//! files whose only job is to make the model read enough text for the context to grow.
//! Nothing in it states a planted fact (port, rule, file name), so an answer cannot be read
//! back off disk, except the ones the scenario itself edits in. The hard fixture adds an
//! AGENTS.md rule, much larger generated files, and two values that exist only in a tool result.

use std::path::Path;

use anyhow::{Context, Result};

use super::scenario::FixtureKind;

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

const AGENTS_MD: &str = "# Project rules\n\n- Every new function needs a doc comment whose first line starts with `Forge:`, for example `/// Forge: parses one header line.`\n- Do not add dependencies without asking.\n";

const RUN_CHECKS_SH: &str = "#!/usr/bin/env bash\nfor i in $(seq 1 220); do\n  n=$(printf '%03d' \"$i\")\n  if [ \"$i\" -eq 137 ]; then\n    echo \"FAIL case_$n: checksum mismatch expected=0xBEEF42 got=0xC0FFEE\"\n  else\n    echo \"ok case_$n (0.0$((i % 9 + 1))s)\"\n  fi\ndone\necho \"summary: 219 passed, 1 failed\"\nexit 1\n";

/// 400 rows; the quota of `eu-west-9` (row 233) appears nowhere else.
fn region_table() -> String {
    (0..400)
        .map(|i| {
            if i == 233 {
                "eu-west-9,quota=7311,tier=premium\n".to_string()
            } else {
                format!(
                    "region-{i:03},quota={},tier={}\n",
                    (i * 97 + 13) % 6000 + 1000,
                    ["standard", "premium", "trial"][i % 3]
                )
            }
        })
        .collect()
}

/// 700 log lines, about 12k tokens.
fn events_log() -> String {
    let levels = ["INFO", "INFO", "WARN", "INFO", "ERROR", "INFO"];
    (0..700)
        .map(|i| {
            format!(
                "2026-10-{:02} {:02}:{:02}:{:02} level={} component=svc-{} msg=\"handled request {} in {} ms\"\n",
                i % 28 + 1,
                i % 12 + 10,
                i % 60,
                (i * 7) % 60,
                levels[i % levels.len()],
                i % 11,
                i,
                (i * 13) % 250
            )
        })
        .collect()
}

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

fn files(kind: FixtureKind) -> Vec<(&'static str, String)> {
    let mut files = vec![
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
    ];
    if kind == FixtureKind::Hard {
        files.extend([
            ("AGENTS.md", AGENTS_MD.to_string()),
            ("scripts/run_checks.sh", RUN_CHECKS_SH.to_string()),
            ("data/region_table.csv", region_table()),
            ("data/events.log", events_log()),
        ]);
    }
    files
}

fn git(dir: &Path, args: &[&str]) -> Result<()> {
    crate::bench::run_git(dir, args).map(|_| ())
}

/// Write the fixture into `dir` and make it a git repo with one baseline commit, so the
/// "commit your changes" probe has something to commit onto.
pub(super) fn create(dir: &Path, kind: FixtureKind) -> Result<()> {
    for (rel, body) in files(kind) {
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

    const PLANTED: [&str; 8] = [
        "47123",
        "settings.toml",
        "JIRA-123",
        "validate_header_len",
        "retry_budget",
        "next_token",
        "03:15",
        "0.4.2",
    ];

    #[test]
    fn the_fixtures_build_committed_repos_without_stating_any_planted_fact() {
        for kind in [FixtureKind::Standard, FixtureKind::Hard] {
            let tmp = tempfile::tempdir().unwrap();
            create(tmp.path(), kind).unwrap();
            assert!(tmp.path().join("src/parser.rs").is_file());
            let log = crate::bench::run_git(tmp.path(), &["log", "--format=%s"]).unwrap();
            assert_eq!(log.trim(), "baseline");
            for (rel, body) in files(kind) {
                for planted in PLANTED {
                    // The standard fixture's Cargo.toml predates the 0.4.2 bump; 0.4.1 is fine.
                    assert!(!body.contains(planted), "{rel} must not contain {planted}");
                }
            }
        }
        assert!(
            !PARSER_RS.contains("unwrap"),
            "the baseline parser obeys its own rule"
        );
        assert!(status_codes().contains("0042,STATUS_0042"));
    }

    #[test]
    fn the_hard_fixture_holds_the_agents_rule_and_the_tool_only_values_exactly_once() {
        let hard = files(FixtureKind::Hard);
        let standard = files(FixtureKind::Standard);
        assert!(standard.iter().all(|(rel, _)| *rel != "AGENTS.md"));
        let all: String = hard.iter().map(|(_, b)| b.as_str()).collect();
        assert!(AGENTS_MD.contains("Forge:"));
        assert_eq!(
            all.matches("7311").count(),
            1,
            "the eu-west-9 quota is unique"
        );
        assert_eq!(all.matches("BEEF42").count(), 1);
        assert!(region_table().lines().count() == 400);
        let big: usize = ["data/region_table.csv", "data/events.log"]
            .iter()
            .map(|name| hard.iter().find(|(r, _)| r == name).unwrap().1.len())
            .sum();
        assert!(
            big > 60_000,
            "bulk files must be large enough to stress pruning: {big}"
        );
    }
}
