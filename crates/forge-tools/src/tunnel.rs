//! Public tunnels (cloudflared / ngrok) that put a loopback port on an `https://` URL.
//!
//! Shared by `/remote` + `forge serve --tunnel` (forge-cli, which owns the remote-control server)
//! and by `shell_job {action:"expose"}` (preview URLs for a dev server the agent started). The
//! caller passes the fixed-tunnel settings as a plain [`TunnelConfig`] so this crate needs no
//! config dependency.

use tokio::sync::mpsc;

/// The fixed-tunnel settings from `[remote]`: both `None` = a quick tunnel with a random URL.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TunnelConfig {
    pub tunnel_name: Option<String>,
    pub tunnel_hostname: Option<String>,
}

/// A public-tunnel provider Forge can drive if it's installed. Probed in priority order: the
/// first one found on `PATH` is used. Each is free to run for a session and gives HTTPS (the
/// page's JS auto-picks `wss://`); both proxy the HTTP WebSocket upgrade transparently, so the
/// existing control page + token gate work unchanged.
///
/// `bore` was deliberately removed from the probe list: its tunnel is raw TCP with no TLS, so
/// the path token, snapshots (source code, cwd, transcript), and permission approvals would all
/// travel the public internet in cleartext — a sniffed token lets a stranger drive the session
/// and approve shell commands (RCE). Its `http://` origin also breaks the PWA (no secure
/// context → no service worker, no notifications).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TunnelKind {
    /// `cloudflared tunnel --url http://localhost:PORT` → `https://<rand>.trycloudflare.com`.
    /// Free, no account, HTTPS, supports WebSocket. Preferred.
    Cloudflared,
    /// `ngrok http PORT` → `https://<id>.ngrok-free.app` (needs a one-time `ngrok config add-authtoken`).
    Ngrok,
}

impl TunnelKind {
    /// All providers in probe priority order.
    pub const ALL: [Self; 2] = [Self::Cloudflared, Self::Ngrok];

    /// The binary name to look for on `PATH`.
    pub fn binary(self) -> &'static str {
        match self {
            Self::Cloudflared => "cloudflared",
            Self::Ngrok => "ngrok",
        }
    }

    /// A one-line human label for scrollback notes.
    pub fn label(self) -> &'static str {
        match self {
            Self::Cloudflared => "cloudflared (trycloudflare.com)",
            Self::Ngrok => "ngrok",
        }
    }

    /// Build the argv that points the tunnel at `local_port`.
    pub fn argv(self, local_port: u16) -> Vec<String> {
        match self {
            Self::Cloudflared => vec![
                "tunnel".into(),
                "--url".into(),
                format!("http://localhost:{local_port}"),
            ],
            Self::Ngrok => vec!["http".into(), local_port.to_string()],
        }
    }

    /// Pull the public URL out of a line of the tunnel's stdout/stderr. Each provider prints it
    /// differently; these patterns are matched against the *verified* output formats:
    /// - cloudflared logs the `https://…trycloudflare.com` URL in a log line on stderr.
    /// - ngrok prints `Forwarding  https://<id>.ngrok-free.app -> http://localhost:PORT`.
    pub fn parse_url(self, line: &str) -> Option<String> {
        match self {
            Self::Cloudflared => {
                // e.g. `... INF +-----------------------------------------+` then a line with the URL,
                // or `Your quick Tunnel has been created ... https://x.trycloudflare.com`. Match any
                // trycloudflare.com https URL on the line.
                line.split_whitespace()
                    .find(|tok| tok.starts_with("https://") && tok.contains("trycloudflare.com"))
                    .map(|t| {
                        t.trim_matches(|c: char| {
                            !c.is_ascii_alphanumeric()
                                && c != ':'
                                && c != '/'
                                && c != '.'
                                && c != '-'
                        })
                        .to_string()
                    })
            }
            Self::Ngrok => {
                // `Forwarding  https://abc.ngrok-free.app -> http://localhost:8080`
                line.split_whitespace()
                    .find(|tok| {
                        tok.starts_with("https://")
                            && (tok.contains("ngrok.io")
                                || tok.contains("ngrok-free.app")
                                || tok.contains("ngrok.app"))
                    })
                    .map(|t| t.trim_end_matches(',').to_string())
            }
        }
    }
}

/// Which tunnel provider (if any) is installed and on `PATH`. Probes each in priority order.
pub fn detect_tunnel() -> Option<TunnelKind> {
    TunnelKind::ALL
        .into_iter()
        .find(|k| which(k.binary()).is_some())
}

/// Which provider a fixed-tunnel config implies, if any. `tunnel_name` is a cloudflared-only
/// concept (named tunnels), so setting it always means cloudflared. A bare `tunnel_hostname`
/// (no `tunnel_name`) means ngrok's reserved-domain flow — a cloudflared *quick* tunnel can't
/// pin a hostname without first creating + DNS-routing a named tunnel, so `tunnel_hostname`
/// alone can only be honored by ngrok. `None` = no fixed-tunnel preference (quick-tunnel
/// behavior, unchanged) — the caller falls back to [`detect_tunnel`]'s PATH-priority pick.
pub fn preferred_tunnel_kind(cfg: &TunnelConfig) -> Option<TunnelKind> {
    if cfg.tunnel_name.is_some() {
        Some(TunnelKind::Cloudflared)
    } else if cfg.tunnel_hostname.is_some() {
        Some(TunnelKind::Ngrok)
    } else {
        None
    }
}

/// Pick the tunnel provider to run for `--anywhere`: the config's fixed-tunnel preference (see
/// [`preferred_tunnel_kind`]) if one is set — erroring with an actionable message if that
/// provider isn't installed — else whichever provider [`detect_tunnel`] finds first on `PATH`.
pub fn resolve_tunnel_kind(cfg: &TunnelConfig) -> std::io::Result<TunnelKind> {
    if let Some(pref) = preferred_tunnel_kind(cfg) {
        if which(pref.binary()).is_some() {
            return Ok(pref);
        }
        let configured = match pref {
            TunnelKind::Cloudflared => "[remote] tunnel_name",
            TunnelKind::Ngrok => "[remote] tunnel_hostname (without tunnel_name)",
        };
        let installed_hint = match detect_tunnel() {
            Some(found) if found != pref => format!(
                " — {} IS installed; either install {}, or drop the fixed-tunnel config to use \
                 {}'s quick tunnel instead",
                found.label(),
                pref.binary(),
                found.label()
            ),
            _ => {
                " and no tunnel tool is installed at all — install cloudflared or ngrok".to_string()
            }
        };
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!(
                "{configured} is set (wants {}), but `{}` is not on PATH{installed_hint}",
                pref.label(),
                pref.binary(),
            ),
        ));
    }
    detect_tunnel().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no tunnel tool found on PATH — install cloudflared or ngrok \
             (bore is unsupported: its tunnel has no TLS, so the access token would travel \
             the public internet in cleartext)",
        )
    })
}

/// How [`spawn_tunnel`] decides the tunnel is up and ready.
#[derive(Debug)]
pub enum TunnelReadiness {
    /// Parse the public URL out of the child's stdout/stderr — the quick-tunnel and ngrok
    /// reserved-domain behavior (both print a URL/Forwarding line).
    ParseUrl,
    /// A cloudflared *named* tunnel run prints no public URL at all (the DNS route is
    /// pre-configured) — instead it logs one "registered tunnel connection" line per edge
    /// connection once live. The public URL is already known from config (`tunnel_hostname`).
    ConnectionRegistered { public_url: String },
}

impl TunnelReadiness {
    /// The public URL once `line` shows the tunnel is live, else `None`.
    pub fn ready_url(&self, kind: TunnelKind, line: &str) -> Option<String> {
        match self {
            Self::ParseUrl => kind.parse_url(line),
            Self::ConnectionRegistered { public_url } => {
                is_cloudflared_connection_registered(line).then(|| public_url.clone())
            }
        }
    }
}

/// Whether `line` is a cloudflared "edge connection registered" log line — printed once per
/// connection (cloudflared opens several) once a `tunnel run` (quick or named) is actually
/// live. This is cloudflared's documented connector log phrasing ("Registered tunnel
/// connection connIndex=…"); it has not been exercised against a live cloudflared process in
/// this repo (no cloudflare account available — see the crate's test module for the covered
/// cases). Matched case-insensitively and loosely (substring, not a strict prefix) so minor
/// spacing/timestamp/log-level formatting changes across cloudflared releases don't silently
/// break readiness detection; `connindex=` alone is also matched as a fallback in case the
/// surrounding words ever change.
pub fn is_cloudflared_connection_registered(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    lower.contains("registered tunnel connection") || lower.contains("connindex=")
}

/// Build the argv + readiness signal for `kind` given `local_port` and the `[remote]` tunnel
/// config. Pure (no I/O), so the arg matrix is directly unit-testable. Errors only on
/// misconfiguration (`tunnel_name` set without `tunnel_hostname`) — provider/binary
/// availability is [`resolve_tunnel_kind`]'s job, not this one's.
pub fn tunnel_plan(
    kind: TunnelKind,
    local_port: u16,
    cfg: &TunnelConfig,
) -> std::io::Result<(Vec<String>, TunnelReadiness)> {
    match kind {
        TunnelKind::Cloudflared => {
            if let Some(name) = cfg.tunnel_name.as_deref() {
                let hostname = cfg.tunnel_hostname.as_deref().ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        format!(
                            "[remote] tunnel_name = \"{name}\" is set but tunnel_hostname is \
                             missing — a named cloudflared tunnel needs a DNS-routed hostname to \
                             advertise. One-time setup:\n  \
                             1. cloudflared tunnel login\n  \
                             2. cloudflared tunnel create {name}\n  \
                             3. cloudflared tunnel route dns {name} <your-hostname>\n\
                             then set tunnel_hostname = \"<your-hostname>\" in [remote]."
                        ),
                    )
                })?;
                let argv = vec![
                    "tunnel".to_string(),
                    "--url".to_string(),
                    format!("http://localhost:{local_port}"),
                    "run".to_string(),
                    name.to_string(),
                ];
                Ok((
                    argv,
                    TunnelReadiness::ConnectionRegistered {
                        public_url: format!("https://{hostname}"),
                    },
                ))
            } else {
                Ok((kind.argv(local_port), TunnelReadiness::ParseUrl))
            }
        }
        TunnelKind::Ngrok => {
            if let Some(hostname) = cfg.tunnel_hostname.as_deref() {
                let argv = vec![
                    "http".to_string(),
                    format!("--domain={hostname}"),
                    local_port.to_string(),
                ];
                Ok((argv, TunnelReadiness::ParseUrl))
            } else {
                Ok((kind.argv(local_port), TunnelReadiness::ParseUrl))
            }
        }
    }
}

/// Best-effort `which`: is `bin` resolvable on `PATH`? Uses `std::env::var` + a manual search so
/// we don't pull a `which` crate; on Windows it also checks for `.exe`/`.cmd`/`.bat` suffixes.
pub fn which(bin: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    let exts = if cfg!(windows) {
        vec!["", ".exe", ".cmd", ".bat"]
    } else {
        vec![""]
    };
    for dir in std::env::split_paths(&path) {
        for ext in &exts {
            let candidate = dir.join(format!("{bin}{ext}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Bounded tail of the tunnel's recent combined stdout/stderr lines, kept only so a
/// named-cloudflared-tunnel failure can show *why* — the quick-tunnel/ngrok error paths don't
/// use it, so their wording is unchanged from before this tail existed.
const TAIL_LINES: usize = 20;

fn push_tail(tail: &mut std::collections::VecDeque<String>, line: &str) {
    if tail.len() == TAIL_LINES {
        tail.pop_front();
    }
    tail.push_back(line.to_string());
}

/// Spawn a tunnel of `kind` pointing at `local_port`, honoring the `[remote]` fixed-tunnel
/// config (`tunnel_name`/`tunnel_hostname` — see [`tunnel_plan`]). Returns the public URL + the
/// child handle (so the caller can kill it when remote control turns off). Fails if the child
/// can't start, is misconfigured (named tunnel without a hostname), or never signals readiness
/// within the timeout (the tunnel is then killed).
pub async fn spawn_tunnel(
    kind: TunnelKind,
    local_port: u16,
    remote_cfg: &TunnelConfig,
) -> std::io::Result<(String, tokio::process::Child)> {
    use tokio::io::AsyncReadExt;

    let bin = which(kind.binary()).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("{} not on PATH", kind.binary()),
        )
    })?;
    let (argv, readiness) = tunnel_plan(kind, local_port, remote_cfg)?;
    let mut cmd = tokio::process::Command::new(bin);
    cmd.args(argv)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd.spawn()?;

    // Merge stdout + stderr so the URL/readiness line (whichever stream it lands on) is seen.
    // cloudflared prints on stderr; ngrok on stdout. Read both concurrently.
    // The readers drain to EOF (the child's exit) regardless of whether anyone is still receiving:
    // once we're done waiting we stop reading `rx`, but a chatty tunnel keeps logging — if we
    // stopped draining its pipe, a full pipe buffer would block the tunnel process and stall
    // forwarding.
    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
    // Generous buffer: the URL/readiness line appears within the first handful of log lines, but
    // the receiver may not be polling yet — a deep buffer means an early burst can't drop it.
    let (tx, mut rx) = mpsc::channel::<String>(256);

    let tx1 = tx.clone();
    tokio::spawn(async move {
        let mut buf = [0u8; 4096];
        loop {
            match stdout.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let chunk = String::from_utf8_lossy(&buf[..n]).to_string();
                    for line in chunk.lines() {
                        // Non-blocking: drop the line if the buffer is full or rx is gone, but NEVER
                        // block the reader — a blocked reader stops draining the pipe (deadlock).
                        let _ = tx1.try_send(line.to_string());
                    }
                }
            }
        }
    });
    tokio::spawn(async move {
        let mut buf = [0u8; 4096];
        loop {
            match stderr.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let chunk = String::from_utf8_lossy(&buf[..n]).to_string();
                    for line in chunk.lines() {
                        // Non-blocking (see stdout reader): keep draining stderr to EOF regardless.
                        let _ = tx.try_send(line.to_string());
                    }
                }
            }
        }
    });

    let mut tail: std::collections::VecDeque<String> = std::collections::VecDeque::new();

    // Wait up to 20s for readiness. Tunnels take a few seconds to register; 20s is generous
    // without hanging forever on a broken/misconfigured install.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            let _ = child.kill().await;
            return Err(match &readiness {
                TunnelReadiness::ParseUrl => std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    format!("{} did not print a public URL within 20s", kind.binary()),
                ),
                TunnelReadiness::ConnectionRegistered { .. } => std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    format!(
                        "{} did not register a tunnel connection within 20s (named tunnel run)\n\
                         --- last output ---\n{}",
                        kind.binary(),
                        tail.iter().cloned().collect::<Vec<_>>().join("\n")
                    ),
                ),
            });
        }
        match tokio::time::timeout(remaining, rx.recv()).await {
            Ok(Some(line)) => {
                push_tail(&mut tail, &line);
                if let Some(url) = readiness.ready_url(kind, &line) {
                    return Ok((url, child));
                }
            }
            Ok(None) => break, // both readers closed (child exited early)
            Err(_) => {}       // timeout on this recv; loop checks the deadline
        }
    }
    let status = child.try_wait().ok().flatten();
    let _ = child.kill().await;
    Err(match &readiness {
        TunnelReadiness::ParseUrl => std::io::Error::other(format!(
            "{} exited before printing a URL{}",
            kind.binary(),
            status.map(|s| format!(" (status {s})")).unwrap_or_default()
        )),
        TunnelReadiness::ConnectionRegistered { .. } => std::io::Error::other(format!(
            "{} exited before registering a tunnel connection{}\n--- last output ---\n{}",
            kind.binary(),
            status.map(|s| format!(" (status {s})")).unwrap_or_default(),
            tail.into_iter().collect::<Vec<_>>().join("\n")
        )),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloudflared_url_parses_from_a_log_line() {
        // cloudflared prints the quick-tunnel URL in a boxed stderr log line.
        let line = "2026-06-18T17:00:00Z INF |  https://random-words-here.trycloudflare.com  |";
        assert_eq!(
            TunnelKind::Cloudflared.parse_url(line).as_deref(),
            Some("https://random-words-here.trycloudflare.com")
        );
        // A non-URL log line yields nothing.
        assert_eq!(
            TunnelKind::Cloudflared.parse_url("INF Starting tunnel"),
            None
        );
    }

    #[test]
    fn ngrok_url_parses_from_forwarding_line() {
        let line = "Forwarding   https://abc123.ngrok-free.app -> http://localhost:8080";
        assert_eq!(
            TunnelKind::Ngrok.parse_url(line).as_deref(),
            Some("https://abc123.ngrok-free.app")
        );
        // Legacy ngrok.io domain still matches.
        assert_eq!(
            TunnelKind::Ngrok
                .parse_url("Forwarding https://x.ngrok.io -> localhost")
                .as_deref(),
            Some("https://x.ngrok.io")
        );
    }

    #[test]
    fn tunnel_argv_points_at_the_local_port() {
        assert_eq!(
            TunnelKind::Cloudflared.argv(8080),
            vec!["tunnel", "--url", "http://localhost:8080"]
        );
        assert_eq!(TunnelKind::Ngrok.argv(8080), vec!["http", "8080"]);
    }

    #[test]
    fn preferred_tunnel_kind_reads_config() {
        let none = TunnelConfig::default();
        assert_eq!(preferred_tunnel_kind(&none), None);

        let named = TunnelConfig {
            tunnel_name: Some("forge".into()),
            tunnel_hostname: Some("forge.example.com".into()),
        };
        assert_eq!(preferred_tunnel_kind(&named), Some(TunnelKind::Cloudflared));

        // Hostname alone (no tunnel_name) implies ngrok's reserved-domain flow — cloudflared
        // can't pin a hostname without first creating + DNS-routing a named tunnel.
        let hostname_only = TunnelConfig {
            tunnel_name: None,
            tunnel_hostname: Some("forge.example.com".into()),
        };
        assert_eq!(
            preferred_tunnel_kind(&hostname_only),
            Some(TunnelKind::Ngrok)
        );
    }

    #[test]
    fn tunnel_plan_quick_tunnel_when_unconfigured() {
        let cfg = TunnelConfig::default();
        let (argv, readiness) = tunnel_plan(TunnelKind::Cloudflared, 8080, &cfg).unwrap();
        assert_eq!(argv, vec!["tunnel", "--url", "http://localhost:8080"]);
        assert!(matches!(readiness, TunnelReadiness::ParseUrl));

        let (argv, readiness) = tunnel_plan(TunnelKind::Ngrok, 8080, &cfg).unwrap();
        assert_eq!(argv, vec!["http", "8080"]);
        assert!(matches!(readiness, TunnelReadiness::ParseUrl));
    }

    #[test]
    fn tunnel_plan_named_cloudflared_tunnel() {
        let cfg = TunnelConfig {
            tunnel_name: Some("forge".into()),
            tunnel_hostname: Some("forge.example.com".into()),
        };
        let (argv, readiness) = tunnel_plan(TunnelKind::Cloudflared, 8080, &cfg).unwrap();
        assert_eq!(
            argv,
            vec!["tunnel", "--url", "http://localhost:8080", "run", "forge"]
        );
        match readiness {
            TunnelReadiness::ConnectionRegistered { public_url } => {
                assert_eq!(public_url, "https://forge.example.com");
            }
            TunnelReadiness::ParseUrl => panic!("named tunnel must wait on connection, not a URL"),
        }
    }

    #[test]
    fn tunnel_plan_named_cloudflared_tunnel_without_hostname_errors() {
        let cfg = TunnelConfig {
            tunnel_name: Some("forge".into()),
            tunnel_hostname: None,
        };
        let err = tunnel_plan(TunnelKind::Cloudflared, 8080, &cfg).unwrap_err();
        let msg = err.to_string();
        // The error must be actionable: name the 3 one-time setup commands.
        assert!(msg.contains("cloudflared tunnel login"), "{msg}");
        assert!(msg.contains("cloudflared tunnel create forge"), "{msg}");
        assert!(msg.contains("cloudflared tunnel route dns forge"), "{msg}");
    }

    #[test]
    fn tunnel_plan_ngrok_reserved_domain() {
        let cfg = TunnelConfig {
            tunnel_name: None,
            tunnel_hostname: Some("forge.example.com".into()),
        };
        let (argv, readiness) = tunnel_plan(TunnelKind::Ngrok, 8080, &cfg).unwrap();
        assert_eq!(argv, vec!["http", "--domain=forge.example.com", "8080"]);
        assert!(matches!(readiness, TunnelReadiness::ParseUrl));
    }

    #[test]
    fn cloudflared_connection_registered_line_matches() {
        // Canned cloudflared connector log line (this codebase has no cloudflare account to
        // capture a real one from — see is_cloudflared_connection_registered's doc comment).
        let line = "2026-07-11T12:00:00Z INF Registered tunnel connection connIndex=0 \
                     connection=abc event=0 ip=198.51.100.1 location=AMS protocol=quic";
        assert!(is_cloudflared_connection_registered(line));
        // Case-insensitive.
        assert!(is_cloudflared_connection_registered(
            "INF registered tunnel connection connIndex=1"
        ));
        // Unrelated log lines don't match.
        assert!(!is_cloudflared_connection_registered(
            "INF Starting tunnel daemon"
        ));
        assert!(!is_cloudflared_connection_registered(
            "INF Initial protocol quic"
        ));
    }

    #[test]
    fn resolve_tunnel_kind_falls_back_to_detect_when_unconfigured() {
        // With no fixed-tunnel config, resolution must not error just because a specific binary
        // is missing — it defers entirely to `detect_tunnel` (PATH probing), which itself
        // returns `None` gracefully rather than panicking in a CI sandbox with neither tool
        // installed. This test only exercises the no-config branch, not PATH contents.
        let cfg = TunnelConfig::default();
        assert_eq!(preferred_tunnel_kind(&cfg), None);
        // resolve_tunnel_kind(&cfg) either finds a real PATH tool or errors "no tunnel tool
        // found" — both are valid depending on the CI image, so just assert it doesn't panic.
        let _ = resolve_tunnel_kind(&cfg);
    }

    #[test]
    fn bore_is_not_probed_as_a_tunnel_provider() {
        // bore forwards raw TCP (no TLS): the token + transcript + approvals would cross the
        // public internet in cleartext. It must never be in the auto-detect list.
        assert!(!TunnelKind::ALL.iter().any(|k| k.binary() == "bore"));
    }
}
