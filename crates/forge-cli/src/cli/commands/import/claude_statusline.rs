//! Carry Claude Code's `statusLine` setting into Forge's `[statusline] command`.

use anyhow::{Context, Result};

const START: &str = "# BEGIN Forge import: Claude Code statusLine\n";
const END: &str = "# END Forge import: Claude Code statusLine\n";

/// The `statusLine.command` of the first source that defines one (project settings are listed
/// after user settings, so the last definition wins, as in Claude Code).
pub(super) fn cc_statusline_command(values: &[serde_json::Value]) -> Option<String> {
    values
        .iter()
        .filter_map(|v| {
            let sl = v.get("statusLine")?;
            if sl.get("type").and_then(|t| t.as_str()).unwrap_or("command") != "command" {
                return None;
            }
            let cmd = sl.get("command")?.as_str()?.trim();
            (!cmd.is_empty()).then(|| cmd.to_string())
        })
        .next_back()
}

/// What importing did, for the summary line.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Outcome {
    Written,
    /// The config already has its own `[statusline]` table; left alone rather than guessed at.
    KeptExisting,
}

/// Write `[statusline] command` into `config_dst` inside an importer-owned block, so a re-import
/// replaces it instead of duplicating it. A `[statusline]` table the user wrote is never touched.
pub(super) fn write_statusline_command(
    command: &str,
    config_dst: &std::path::Path,
) -> Result<Outcome> {
    let existing = match std::fs::read_to_string(config_dst) {
        Ok(text) => text,
        Err(_) => "# Forge config\n".to_string(),
    };
    let stripped = match (existing.find(START), existing.find(END)) {
        (Some(start), Some(end)) if end >= start => {
            format!("{}{}", &existing[..start], &existing[end + END.len()..])
        }
        (None, None) => existing,
        _ => anyhow::bail!(
            "malformed imported statusLine block in {}",
            config_dst.display()
        ),
    };
    let parsed: toml::Table =
        toml::from_str(&stripped).with_context(|| format!("parsing {}", config_dst.display()))?;
    if parsed.contains_key("statusline") {
        return Ok(Outcome::KeptExisting);
    }
    if let Some(parent) = config_dst.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let value = toml::Value::String(command.to_string());
    let mut out = stripped;
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&format!("{START}[statusline]\ncommand = {value}\n{END}"));
    std::fs::write(config_dst, out).with_context(|| format!("writing {}", config_dst.display()))?;
    Ok(Outcome::Written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_command_type_and_last_source() {
        let user = serde_json::json!({"statusLine":{"type":"command","command":"a.sh"}});
        let project = serde_json::json!({"statusLine":{"type":"command","command":"b.sh"}});
        assert_eq!(
            cc_statusline_command(&[user.clone(), project]).as_deref(),
            Some("b.sh")
        );
        let other = serde_json::json!({"statusLine":{"type":"static","command":"x"}});
        assert_eq!(cc_statusline_command(&[other]), None);
        assert_eq!(cc_statusline_command(&[serde_json::json!({})]), None);
    }

    #[test]
    fn write_is_idempotent_and_loads_back() {
        let dir = std::env::temp_dir().join(format!("forge-sl-import-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dst = dir.join("config.toml");
        std::fs::write(&dst, "[tui]\nx = 1\n").unwrap();
        for _ in 0..2 {
            assert_eq!(
                write_statusline_command("~/.claude/statusline.sh \"q\"", &dst).unwrap(),
                Outcome::Written
            );
        }
        let text = std::fs::read_to_string(&dst).unwrap();
        assert_eq!(text.matches("[statusline]").count(), 1);
        let cfg: toml::Table = toml::from_str(&text).unwrap();
        assert_eq!(
            cfg["statusline"]["command"].as_str(),
            Some("~/.claude/statusline.sh \"q\"")
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn user_statusline_table_is_kept() {
        let dir = std::env::temp_dir().join(format!("forge-sl-keep-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dst = dir.join("config.toml");
        std::fs::write(&dst, "[statusline]\nseparator = \"|\"\n").unwrap();
        assert_eq!(
            write_statusline_command("s.sh", &dst).unwrap(),
            Outcome::KeptExisting
        );
        assert!(!std::fs::read_to_string(&dst).unwrap().contains("s.sh"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
