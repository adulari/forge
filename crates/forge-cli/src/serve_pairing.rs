//! Pairing trust for the daemon: a persisted LAN certificate whose leaf fingerprint rides in the
//! pairing link, `_forge._tcp` mDNS advertising for the Nearby list, and Tailscale detection.

use crate::remote::SelfSignedCert;
use anyhow::{Context, Result};
use serde::Serialize;
use std::path::Path;

const CERT_FILE: &str = "serve-tls-cert.pem";
const KEY_FILE: &str = "serve-tls-key.pem";
const SANS_FILE: &str = "serve-tls-sans.txt";
pub(crate) const MDNS_SERVICE: &str = "_forge._tcp.local.";

/// `AB:CD:..` as printed by the TLS banner to the `sha256:abcd..` form carried in links.
pub(crate) fn link_fingerprint(colon_hex: &str) -> String {
    let hex: String = colon_hex
        .chars()
        .filter(char::is_ascii_hexdigit)
        .collect::<String>()
        .to_ascii_lowercase();
    format!("sha256:{hex}")
}

/// The pairing link for an origin. With a leaf fingerprint the token and pin travel in the URL
/// fragment so neither reaches a proxy or access log; without one (a tunnel or Tailscale
/// certificate that browsers already trust) it is the classic path form.
pub(crate) fn pairing_link(origin: &str, token: &str, fingerprint: Option<&str>) -> String {
    let origin = origin.trim_end_matches('/');
    match fingerprint {
        Some(fp) => format!("{origin}/#token={token}&fp={}", link_fingerprint(fp)),
        None => format!("{origin}/{token}"),
    }
}

/// Reuse the certificate saved in `dir` while it still covers every requested name, otherwise
/// mint and save a new one. Keeping the leaf stable across restarts is what makes pinning
/// meaningful: a changed fingerprint then really means a different machine or a rotated cert.
pub(crate) fn load_or_create_cert(dir: &Path, sans: Vec<String>) -> Result<SelfSignedCert> {
    let (cert_path, key_path, sans_path) =
        (dir.join(CERT_FILE), dir.join(KEY_FILE), dir.join(SANS_FILE));
    let mut wanted = sans.clone();
    wanted.sort();
    wanted.dedup();
    if let (Ok(cert), Ok(key), Ok(saved)) = (
        std::fs::read(&cert_path),
        std::fs::read(&key_path),
        std::fs::read_to_string(&sans_path),
    ) {
        let mut have: Vec<String> = saved.lines().map(str::to_string).collect();
        have.sort();
        if have == wanted {
            if let Some(fingerprint) = fingerprint_of_pem(&cert) {
                return Ok(SelfSignedCert {
                    cert_pem: cert,
                    key_pem: key,
                    fingerprint,
                });
            }
        }
    }
    let fresh = crate::remote::generate_self_signed(sans)
        .map_err(|e| anyhow::anyhow!("self-signed cert generation failed: {e}"))?;
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    std::fs::write(&cert_path, &fresh.cert_pem)?;
    std::fs::write(&sans_path, wanted.join("\n"))?;
    write_private(&key_path, &fresh.key_pem)?;
    Ok(fresh)
}

fn create_private(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options.open(path)
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write as _;
    let mut file = create_private(path).with_context(|| format!("writing {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(bytes)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

pub(crate) fn fingerprint_of_pem(pem: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(pem).ok()?;
    let body: String = text
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .collect::<String>();
    let der = base64_decode(&body)?;
    Some(crate::remote::sha256_fingerprint(&der))
}

fn base64_decode(text: &str) -> Option<Vec<u8>> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.decode(text).ok()
}

/// Keeps the advertisement alive; dropping it withdraws the service.
pub(crate) struct Advertisement(mdns_sd::ServiceDaemon);

impl Drop for Advertisement {
    fn drop(&mut self) {
        let _ = self.0.shutdown();
    }
}

/// Announce this daemon as `_forge._tcp` with TXT `id` and `fp`. The token never goes in TXT:
/// anything on the LAN can read it.
pub(crate) fn advertise(port: u16, host_id: &str, fingerprint: &str) -> Option<Advertisement> {
    let daemon = mdns_sd::ServiceDaemon::new().ok()?;
    let host = format!("{}.local.", sanitize_label(host_id));
    let fp = link_fingerprint(fingerprint);
    let properties = [("id", host_id), ("fp", fp.as_str())];
    let info = mdns_sd::ServiceInfo::new(MDNS_SERVICE, host_id, &host, "", port, &properties[..])
        .ok()?
        .enable_addr_auto();
    daemon.register(info).ok()?;
    Some(Advertisement(daemon))
}

fn sanitize_label(text: &str) -> String {
    let label: String = text
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let label = label.trim_matches('-');
    if label.is_empty() {
        "forge".into()
    } else {
        label.to_string()
    }
}

pub(crate) fn host_id() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "forge".into())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub(crate) enum Tailscale {
    NotInstalled,
    LoggedOut,
    Stopped,
    NoHttps { dns_name: String },
    Ready { dns_name: String },
}

impl Tailscale {
    pub(crate) fn note(&self) -> String {
        match self {
            Tailscale::NotInstalled => "Tailscale was not detected on this machine.".into(),
            Tailscale::LoggedOut => "Tailscale needs you to log in: run `tailscale up`.".into(),
            Tailscale::Stopped => "Tailscale is installed but stopped: run `tailscale up`.".into(),
            Tailscale::NoHttps { .. } => {
                "Tailscale is up, but HTTPS certificates are off for the tailnet. Enable HTTPS in the admin console DNS page.".into()
            }
            Tailscale::Ready { dns_name } => format!(
                "Reachable from your tailnet at {dns_name} with a trusted certificate (`forge serve --tailscale`)."
            ),
        }
    }
}

pub(crate) fn parse_tailscale_status(json: &str) -> Tailscale {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return Tailscale::NotInstalled;
    };
    match value["BackendState"].as_str().unwrap_or("") {
        "NeedsLogin" | "NeedsMachineAuth" => return Tailscale::LoggedOut,
        "Running" => {}
        _ => return Tailscale::Stopped,
    }
    let dns_name = value["Self"]["DNSName"]
        .as_str()
        .unwrap_or("")
        .trim_end_matches('.')
        .to_string();
    let has_cert = value["CertDomains"]
        .as_array()
        .is_some_and(|domains| !domains.is_empty());
    match (dns_name.is_empty(), has_cert) {
        (true, _) => Tailscale::Stopped,
        (false, true) => Tailscale::Ready { dns_name },
        (false, false) => Tailscale::NoHttps { dns_name },
    }
}

pub(crate) fn tailscale_state() -> Tailscale {
    match std::process::Command::new("tailscale")
        .args(["status", "--json"])
        .stderr(std::process::Stdio::null())
        .output()
    {
        Ok(out) if !out.stdout.is_empty() => {
            parse_tailscale_status(&String::from_utf8_lossy(&out.stdout))
        }
        _ => Tailscale::NotInstalled,
    }
}

/// Put the daemon behind `tailscale serve` on 443, returning the MagicDNS origin.
pub(crate) fn tailscale_serve(port: u16) -> Result<String> {
    let dns_name = match tailscale_state() {
        Tailscale::Ready { dns_name } => dns_name,
        other => anyhow::bail!("{}", other.note()),
    };
    let target = format!("https+insecure://127.0.0.1:{port}");
    let out = std::process::Command::new("tailscale")
        .args(["serve", "--bg", "--https=443", &target])
        .output()
        .context("running `tailscale serve`")?;
    if !out.status.success() {
        anyhow::bail!(
            "`tailscale serve` failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(format!("https://{dns_name}"))
}

pub(crate) fn lan_cert(cert_host: Option<&str>) -> Result<Option<SelfSignedCert>> {
    let Some(host) = cert_host else {
        return Ok(None);
    };
    let dir = forge_config::config_dir().context("no config directory on this platform")?;
    load_or_create_cert(&dir, vec![host.to_string(), "localhost".to_string()]).map(Some)
}

/// Link printed and shown as the QR: Tailscale origin, then `[remote] public_url`, then the
/// pinned LAN link, then the plain URL.
pub(crate) fn connect_link(
    tailscale: bool,
    port: u16,
    public_url: Option<&str>,
    lan_origin: &str,
    token: &str,
    lan_fingerprint: Option<&str>,
    fallback: &str,
) -> String {
    if tailscale {
        match tailscale_serve(port) {
            Ok(origin) => {
                println!("  tailscale: serving {origin} (trusted certificate)");
                return pairing_link(&origin, token, None);
            }
            Err(e) => eprintln!("⚠ --tailscale: {e:#}"),
        }
    }
    if let Some(public) = public_url.filter(|u| !u.trim().is_empty()) {
        return pairing_link(public, token, None);
    }
    match lan_fingerprint {
        Some(fp) => pairing_link(lan_origin, token, Some(fp)),
        None => fallback.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_carries_token_and_pin_in_the_fragment() {
        let link = pairing_link("https://10.0.0.5:7420/", "abcd", Some("AB:CD:01"));
        assert_eq!(link, "https://10.0.0.5:7420/#token=abcd&fp=sha256:abcd01");
        assert_eq!(
            pairing_link("https://box.ts.net", "abcd", None),
            "https://box.ts.net/abcd"
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_key_file_is_private_from_the_moment_it_exists() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("key.pem");
        let file = create_private(&path).unwrap();
        let mode = file.metadata().unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode & 0o077,
            0,
            "group/other bits set at creation: {mode:o}"
        );

        let fresh = tempfile::tempdir().unwrap();
        load_or_create_cert(fresh.path(), vec!["localhost".to_string()]).unwrap();
        let key = fresh.path().join(KEY_FILE);
        assert_eq!(
            std::fs::metadata(key).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn saved_cert_survives_restart_and_changes_with_names() {
        let dir = tempfile::tempdir().unwrap();
        let names = vec!["10.0.0.5".to_string(), "localhost".to_string()];
        let first = load_or_create_cert(dir.path(), names.clone()).unwrap();
        let again = load_or_create_cert(dir.path(), names).unwrap();
        assert_eq!(first.fingerprint, again.fingerprint);
        let moved = load_or_create_cert(
            dir.path(),
            vec!["10.0.0.9".to_string(), "localhost".to_string()],
        )
        .unwrap();
        assert_ne!(first.fingerprint, moved.fingerprint);
    }

    #[test]
    fn tailscale_status_maps_to_doctor_states() {
        let ready = r#"{"BackendState":"Running","Self":{"DNSName":"box.tail1.ts.net."},"CertDomains":["box.tail1.ts.net"]}"#;
        assert_eq!(
            parse_tailscale_status(ready),
            Tailscale::Ready {
                dns_name: "box.tail1.ts.net".into()
            }
        );
        let no_https =
            r#"{"BackendState":"Running","Self":{"DNSName":"box.tail1.ts.net."},"CertDomains":[]}"#;
        assert!(matches!(
            parse_tailscale_status(no_https),
            Tailscale::NoHttps { .. }
        ));
        assert_eq!(
            parse_tailscale_status(r#"{"BackendState":"NeedsLogin"}"#),
            Tailscale::LoggedOut
        );
        assert_eq!(
            parse_tailscale_status(r#"{"BackendState":"Stopped"}"#),
            Tailscale::Stopped
        );
        assert_eq!(parse_tailscale_status(""), Tailscale::NotInstalled);
    }
}
