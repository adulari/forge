//! `forge serve pair` and `forge serve clients list|revoke`: the terminal side of the per-client
//! token registry. They edit the shared file directly, so they work with the daemon stopped and a
//! running daemon picks the change up within a couple of seconds.

use crate::cli::args::mcp::serve::{ClientsCmd, ServeCmd};
use crate::serve_clients::{
    now_secs, ClientRecord, ClientRegistry, Preset, DEFAULT_PAIR_TTL_SECS, MAX_PAIR_TTL_SECS,
};
use anyhow::{bail, Context, Result};

pub(crate) fn run(cmd: ServeCmd) -> Result<()> {
    let registry = ClientRegistry::open(Some(
        ClientRegistry::default_path().context("no config directory on this platform")?,
    ));
    match cmd {
        ServeCmd::Pair { ttl, preset, label } => {
            pair(&registry, &ttl, preset.as_deref(), label.as_deref())
        }
        ServeCmd::Clients {
            cmd: ClientsCmd::List,
        } => {
            print!("{}", render_list(&registry.list(), now_secs()));
            Ok(())
        }
        ServeCmd::Clients {
            cmd: ClientsCmd::Revoke { id },
        } => {
            let record = registry.revoke(&id)?;
            println!("revoked {} ({})", record.id, record.label);
            Ok(())
        }
    }
}

fn pair(
    registry: &ClientRegistry,
    ttl: &str,
    preset: Option<&str>,
    label: Option<&str>,
) -> Result<()> {
    let ttl = parse_ttl(ttl)?;
    let preset = match preset {
        None => Preset::Full,
        Some(text) => Preset::parse(text).context("--preset must be `full` or `viewer`")?,
    };
    let (record, token) = registry.mint(label.unwrap_or(""), preset.scopes(), Some(ttl))?;
    let (origin, running) = match crate::serve::read_state() {
        Ok(Some(state)) if state.process_is_alive() => {
            (origin_of(&state.base_url, &state.token), true)
        }
        _ => {
            let port = forge_config::load().unwrap_or_default().remote.serve_port();
            (format!("http://127.0.0.1:{port}"), false)
        }
    };
    let link = match crate::serve::read_state() {
        Ok(Some(state)) if running => {
            let fingerprint = cert_fingerprint_for(&state.exposure);
            crate::serve_pairing::pairing_link(&origin, &token, fingerprint.as_deref())
        }
        _ => format!("{origin}/{token}"),
    };
    println!("paired device {} ({})", record.id, record.label);
    println!("  access: {}", scope_summary(&record));
    println!(
        "  valid for {} to open once; revoke with `forge serve clients revoke {}`",
        human_duration(ttl),
        record.id
    );
    println!("  link:   {link}");
    if !running {
        println!("  (no running daemon found; the link uses the default local address)");
    }
    if std::io::IsTerminal::is_terminal(&std::io::stdout()) {
        for line in crate::remote::qr_lines(&link).unwrap_or_default() {
            println!("{line}");
        }
    }
    Ok(())
}

fn cert_fingerprint_for(exposure: &str) -> Option<String> {
    if exposure != "lan" {
        return None;
    }
    let dir = forge_config::config_dir()?;
    let pem = std::fs::read(dir.join("serve-tls-cert.pem")).ok()?;
    crate::serve_pairing::fingerprint_of_pem(&pem)
}

fn origin_of(base_url: &str, owner_token: &str) -> String {
    base_url
        .trim_end_matches('/')
        .trim_end_matches(owner_token)
        .trim_end_matches('/')
        .to_string()
}

pub(crate) fn parse_ttl(text: &str) -> Result<u64> {
    let text = text.trim();
    let (digits, unit) = match text.char_indices().find(|(_, c)| !c.is_ascii_digit()) {
        Some((i, _)) => text.split_at(i),
        None => (text, "s"),
    };
    let n: u64 = digits
        .parse()
        .with_context(|| format!("`{text}` is not a duration like 90s, 10m or 2h"))?;
    let secs = match unit {
        "s" => n,
        "m" => n * 60,
        "h" => n * 3600,
        "d" => n * 86_400,
        _ => bail!("`{text}` is not a duration like 90s, 10m or 2h"),
    };
    if !(30..=MAX_PAIR_TTL_SECS).contains(&secs) {
        bail!(
            "--ttl must be between 30s and 24h (default {}m)",
            DEFAULT_PAIR_TTL_SECS / 60
        );
    }
    Ok(secs)
}

fn human_duration(secs: u64) -> String {
    match secs {
        s if s % 3600 == 0 => format!("{}h", s / 3600),
        s if s % 60 == 0 => format!("{}m", s / 60),
        s => format!("{s}s"),
    }
}

fn scope_summary(record: &ClientRecord) -> String {
    if record.scopes == Preset::Full.scopes() {
        "full".into()
    } else {
        record
            .scopes
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn ago(secs: u64) -> String {
    match secs {
        s if s < 60 => "just now".into(),
        s if s < 3600 => format!("{}m ago", s / 60),
        s if s < 86_400 => format!("{}h ago", s / 3600),
        s => format!("{}d ago", s / 86_400),
    }
}

pub(crate) fn render_list(records: &[ClientRecord], now: u64) -> String {
    if records.is_empty() {
        return "no paired devices (pair one with `forge serve pair`)\n".into();
    }
    let mut out = format!(
        "{:<10} {:<24} {:<18} {}\n",
        "ID", "LABEL", "LAST CONNECTED", "ACCESS"
    );
    for record in records {
        let seen = match (record.pending_until, record.last_seen) {
            (Some(_), _) => "waiting to pair".to_string(),
            (None, Some(t)) => ago(now.saturating_sub(t)),
            (None, None) => "never".into(),
        };
        out.push_str(&format!(
            "{:<10} {:<24} {:<18} {}\n",
            record.id,
            record.label.chars().take(24).collect::<String>(),
            seen,
            scope_summary(record)
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serve_clients::Scope;

    #[test]
    fn ttl_accepts_units_and_rejects_nonsense() {
        assert_eq!(parse_ttl("10m").unwrap(), 600);
        assert_eq!(parse_ttl("90s").unwrap(), 90);
        assert_eq!(parse_ttl("2h").unwrap(), 7200);
        assert_eq!(parse_ttl("45").unwrap(), 45);
        assert!(parse_ttl("5s").is_err());
        assert!(parse_ttl("9d").is_err());
        assert!(parse_ttl("soon").is_err());
    }

    #[test]
    fn origin_drops_the_owner_token() {
        assert_eq!(
            origin_of("https://192.168.1.5:7420/abc123", "abc123"),
            "https://192.168.1.5:7420"
        );
        assert_eq!(
            origin_of("https://x.example/abc123/", "abc123"),
            "https://x.example"
        );
    }

    #[test]
    fn list_shows_last_connected_and_access() {
        let record = |id: &str, scopes, last_seen, pending_until| ClientRecord {
            id: id.into(),
            label: format!("dev-{id}"),
            token_hash: String::new(),
            scopes,
            created_at: 0,
            last_seen,
            pending_until,
        };
        let text = render_list(
            &[
                record("aaaa1111", Preset::Full.scopes(), Some(970), None),
                record("bbbb2222", vec![Scope::View], None, None),
                record("cccc3333", Preset::Full.scopes(), None, Some(2000)),
            ],
            1000,
        );
        assert!(text.contains("just now") && text.contains("full"));
        assert!(text.contains("never") && text.contains("view"));
        assert!(text.contains("waiting to pair"));
        assert!(render_list(&[], 0).contains("no paired devices"));
    }
}
