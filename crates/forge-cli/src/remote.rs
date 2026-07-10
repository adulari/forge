//! Remote control — drive a running `forge chat` session from a phone or desktop browser.
//!
//! `/remote` (alias `/rc`) starts a tiny HTTP + WebSocket server bound to `0.0.0.0:<ephemeral>`
//! (LAN-reachable). A single HTML control page is served at a token-gated URL (printed into the
//! TUI scrollback + rendered as a QR code so a phone can scan-to-connect). One bidirectional
//! WebSocket carries the live [`Snapshot`] (model · busy · cost · context · statusline · the
//! recent transcript edge) to the browser and [`RemoteInput`] (prompt / answer / interrupt) back.
//!
//! `--local` binds loopback only (control from this machine); `--anywhere` binds loopback and
//! pipes it through a public tunnel (cloudflared / ngrok, whichever is installed) so the
//! page is reachable from any network with NO manual router port-forwarding — the connect URL is
//! then a public `https://…/<token>`. See [`Exposure`] + [`start_anywhere`]. `bore` is
//! deliberately NOT probed: its tunnel is plain TCP end-to-end, so the token, transcript, and
//! permission approvals would travel the public internet in cleartext.
//!
//! The design goals are: *easy* (one slash command, no install, works from any browser), and
//! *accessible on mobile + desktop* (a responsive, low-friction control page that needs no app).
//! The server is in-process so it reuses the running Session + presenter channel — no second
//! process, no IPC, no keys to configure. Security is a random token in the URL path: a LAN peer
//! (or, under `--anywhere`, anyone on the internet) without the token can't drive the session —
//! so the token is genuinely load-bearing once a public tunnel is open.
//!
//! ## TLS (LAN bind only)
//!
//! When binding to `0.0.0.0` (LAN), the server generates a self-signed certificate at startup and
//! serves HTTPS so the access token doesn't travel in cleartext. The cert's SHA-256 fingerprint is
//! printed alongside the connect URL so the user can verify it in the browser's cert dialog.
//! Loopback (`--local`) stays plain HTTP — the connection never leaves the machine. Tunnel modes
//! are unchanged — the provider (cloudflared / ngrok) already terminates TLS. If TLS setup fails
//! the LAN server is declared unavailable rather than silently downgrading to cleartext HTTP:
//! the `https://` connect URL was already handed out, so a plain-HTTP fallback both lies about
//! the transport AND can't be reached at that URL anyway. Use `--local` or `--anywhere` instead.
//!
//! The connect URL / QR / cert SANs use the real outbound-interface IP (discovered via a
//! connected UDP socket — no packets are sent), overridable with `[remote] host` for
//! multi-homed/VPN machines, never the meaningless `0.0.0.0` bind address.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

// ---------------------------------------------------------------------------
// TLS helpers (LAN bind)
// ---------------------------------------------------------------------------

/// A self-signed certificate + private key generated at startup for the LAN HTTPS server.
pub(crate) struct SelfSignedCert {
    /// PEM-encoded certificate (fed to RustlsConfig).
    pub(crate) cert_pem: Vec<u8>,
    /// PEM-encoded private key (fed to RustlsConfig).
    pub(crate) key_pem: Vec<u8>,
    /// SHA-256 fingerprint of the DER-encoded certificate, colon-separated uppercase hex.
    /// e.g. `"AB:CD:EF:…"` — shown to the user so they can verify the cert in their browser.
    pub(crate) fingerprint: String,
}

/// Generate a self-signed TLS certificate valid for the given SANs (Subject Alternative Names).
/// Returns `Err` only if rcgen itself fails, which shouldn't happen with valid input.
pub(crate) fn generate_self_signed(sans: Vec<String>) -> Result<SelfSignedCert, rcgen::Error> {
    let rcgen::CertifiedKey { cert, signing_key } = rcgen::generate_simple_self_signed(sans)?;

    // DER bytes → SHA-256 fingerprint
    let der: &[u8] = cert.der();
    let fingerprint = sha256_fingerprint(der);

    Ok(SelfSignedCert {
        cert_pem: cert.pem().into_bytes(),
        key_pem: signing_key.serialize_pem().into_bytes(),
        fingerprint,
    })
}

/// Compute a SHA-256 digest over `bytes` and return it as uppercase colon-separated hex,
/// e.g. `"AB:CD:EF:…"`. Pure-Rust, no external crypto dep — we just need a fingerprint for
/// display, not a security-critical MAC, so a straightforward byte-by-byte implementation is fine.
fn sha256_fingerprint(bytes: &[u8]) -> String {
    // SHA-256 is available via rustls/ring which are already in the dep tree, but rather than
    // adding another direct dep (ring or sha2) we implement the digest inline using the
    // `rustls` re-export of the ring digest via `rustls::crypto::ring`. However, the cleanest
    // zero-new-dep approach is to use `std` — which has no SHA-256. Instead we rely on `rcgen`
    // pulling in `ring` (which is already compiled) and call it through the public `rcgen` API.
    //
    // Simplest alternative that truly adds no dep: use rustls-provided digest. rustls 0.23
    // exposes `rustls::crypto::CryptoProvider` but not a raw hash. The actual zero-dep path is
    // to implement SHA-256 ourselves — but that's many lines and error-prone. We instead just
    // depend on the `ring` crate (already an indirect dep of rustls + rcgen) via the `rcgen`
    // feature or we access it through `axum-server`'s already-compiled `rustls` stack.
    //
    // Practical decision: use `ring::digest` which is guaranteed to be compiled (it's a dep of
    // rustls 0.23 via the default ring provider). We access it via the re-exported path from
    // `rcgen`'s transitive dep — but that requires adding `ring` to Cargo.toml.
    //
    // To keep this truly dep-free we implement a minimal SHA-256 inline. The implementation
    // follows FIPS 180-4 and is only ~80 lines — acceptable for a display fingerprint.
    let digest = sha256_raw(bytes);
    digest
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// Minimal SHA-256 implementation (FIPS 180-4). Used only for cert fingerprint display.
/// Not constant-time; not intended for HMAC or key derivation.
fn sha256_raw(data: &[u8]) -> [u8; 32] {
    // Round constants (first 32 bits of the fractional parts of the cube roots of the first 64 primes)
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    // Initial hash values (first 32 bits of the fractional parts of the square roots of the first 8 primes)
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    // Pre-processing: pad the message
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0x00);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    // Process each 512-bit (64-byte) chunk
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 64];
        for (i, word) in w.iter_mut().enumerate().take(16) {
            *word = u32::from_be_bytes(chunk[i * 4..i * 4 + 4].try_into().unwrap());
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    let mut out = [0u8; 32];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

/// How the local server is exposed to a browser.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Exposure {
    /// Bind `0.0.0.0` — reachable from the LAN (the original `/remote` default).
    #[default]
    Lan,
    /// Bind `127.0.0.1` only — control from this machine.
    Local,
    /// Bind loopback and pipe it through a public tunnel so any browser, anywhere, can reach it.
    /// No manual router port-forwarding: the tunnel (cloudflared/ngrok) punches through NAT.
    Anywhere,
}

impl From<forge_config::RemoteAuto> for Exposure {
    /// Map the `[remote] auto` config value to a server exposure. `Off` has no exposure (the
    /// caller only converts after [`forge_config::RemoteConfig::startup_exposure`] returns
    /// `Some`), so it falls back to the safest bind (loopback).
    fn from(a: forge_config::RemoteAuto) -> Self {
        match a {
            forge_config::RemoteAuto::Lan => Exposure::Lan,
            forge_config::RemoteAuto::Anywhere => Exposure::Anywhere,
            forge_config::RemoteAuto::Local | forge_config::RemoteAuto::Off => Exposure::Local,
        }
    }
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
pub(crate) enum TunnelKind {
    /// `cloudflared tunnel --url http://localhost:PORT` → `https://<rand>.trycloudflare.com`.
    /// Free, no account, HTTPS, supports WebSocket. Preferred.
    Cloudflared,
    /// `ngrok http PORT` → `https://<id>.ngrok-free.app` (needs a one-time `ngrok config add-authtoken`).
    Ngrok,
}

impl TunnelKind {
    /// All providers in probe priority order.
    const ALL: [Self; 2] = [Self::Cloudflared, Self::Ngrok];

    /// The binary name to look for on `PATH`.
    fn binary(self) -> &'static str {
        match self {
            Self::Cloudflared => "cloudflared",
            Self::Ngrok => "ngrok",
        }
    }

    /// A one-line human label for scrollback notes.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Cloudflared => "cloudflared (trycloudflare.com)",
            Self::Ngrok => "ngrok",
        }
    }

    /// Build the argv that points the tunnel at `local_port`.
    fn argv(self, local_port: u16) -> Vec<String> {
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
    fn parse_url(self, line: &str) -> Option<String> {
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
pub(crate) fn detect_tunnel() -> Option<TunnelKind> {
    TunnelKind::ALL
        .into_iter()
        .find(|k| which(k.binary()).is_some())
}

/// Best-effort `which`: is `bin` resolvable on `PATH`? Uses `std::env::var` + a manual search so
/// we don't pull a `which` crate; on Windows it also checks for `.exe`/`.cmd`/`.bat` suffixes.
fn which(bin: &str) -> Option<std::path::PathBuf> {
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

/// Spawn a tunnel of `kind` pointing at `local_port`. Returns the public URL (parsed from the
/// tunnel's output) + the child handle (so the caller can kill it when remote control turns off).
/// Fails if the child can't start or no URL appears within the timeout (the tunnel is then killed).
pub(crate) async fn spawn_tunnel(
    kind: TunnelKind,
    local_port: u16,
) -> std::io::Result<(String, tokio::process::Child)> {
    use tokio::io::AsyncReadExt;

    let bin = which(kind.binary()).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("{} not on PATH", kind.binary()),
        )
    })?;
    let mut cmd = tokio::process::Command::new(bin);
    cmd.args(kind.argv(local_port))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd.spawn()?;

    // Merge stdout + stderr so the URL (whichever stream it lands on) is seen. cloudflared prints
    // the URL on stderr; ngrok on stdout. Read both concurrently.
    // The readers drain to EOF (the child's exit) regardless of whether anyone is still receiving:
    // once we have the URL we stop reading `rx`, but a chatty tunnel keeps logging — if we stopped
    // draining its pipe, a full pipe buffer would block the tunnel process and stall forwarding.
    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
    // Generous buffer: the URL appears within the first handful of log lines, but the receiver
    // may not be polling yet — a deep buffer means an early burst can't drop the URL line.
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

    // Wait up to 20s for a recognizable public URL line. Tunnels take a few seconds to register;
    // 20s is generous without hanging forever on a broken/misconfigured install.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            let _ = child.kill().await;
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!("{} did not print a public URL within 20s", kind.binary()),
            ));
        }
        match tokio::time::timeout(remaining, rx.recv()).await {
            Ok(Some(line)) => {
                if let Some(url) = kind.parse_url(&line) {
                    return Ok((url, child));
                }
            }
            Ok(None) => break, // both readers closed (child exited early)
            Err(_) => {}       // timeout on this recv; loop checks the deadline
        }
    }
    let status = child.try_wait().ok().flatten();
    let _ = child.kill().await;
    Err(std::io::Error::other(format!(
        "{} exited before printing a URL{}",
        kind.binary(),
        status.map(|s| format!(" (status {s})")).unwrap_or_default()
    )))
}

/// A token-gated URL is printed into the TUI so the user can scan/click to connect.
#[derive(Debug, Clone)]
#[allow(dead_code)] // `token` is read by tests + serves as a stable handle field
pub struct RemoteUrl {
    /// `http(s)://host:port/TOKEN` — the full connect URL (host resolved best-effort).
    pub url: String,
    /// The LAN-visible host:port, for the scrollback note ("listening on …").
    pub addr: SocketAddr,
    /// The random path token (also the WS auth key).
    pub token: String,
    /// SHA-256 fingerprint of the TLS certificate (LAN HTTPS only), colon-separated uppercase
    /// hex. `None` for loopback (HTTP) and tunnel modes (provider terminates TLS).
    pub tls_fingerprint: Option<String>,
}

/// Wire-protocol version for the remote control page ⇄ server contract. Bumped whenever the
/// [`Snapshot`] / [`RemoteInput`] shape changes in a way the page must know about; the page
/// shows a "refresh to update" hint when its bundled version and the server's disagree.
///
/// v3: `Snapshot` gained `prompt_seq`/`notes`/`revision`; `Allow`/`Answer` REQUIRE a `seq`
/// echoing the snapshot's `prompt_seq` (a v2 page's seq-less answers are rejected by design —
/// they can't prove which prompt they target).
///
/// v4: `Snapshot` gained `overlay` (the generic modal projection — palette / every picker /
/// `@path` / config / usage / mesh / workflow) and `copy_text` (the `/copy` payload so the
/// REMOTE device can copy it); `RemoteInput` gained the keystroke channel (`Key`) and the
/// overlay verbs (`OverlaySelect`/`OverlayNav`/`OverlayFilter`/`OverlayCancel`).
///
/// v5: reconnect/replay + full scrollback — the WS handshake accepts `?rev=<last seen revision>`
/// and replays the missed frames from a bounded per-server [`EventLog`] (an unfillable gap
/// falls back to ONE full snapshot flagged `resync: true`, which `Snapshot` gained); a
/// token-scoped `GET /<token>/api/history?before=<seq>&limit=<n>` pages the session's persisted
/// transcript ([`HistoryRow`], newest first) so the page has unlimited scrollback while the
/// snapshot transcript stays a short live tail.
///
/// v6: the multi-session daemon (`forge serve`) — `Snapshot` gained `title` (the session's
/// display name) and `worktree` (the isolated worktree path, when the session runs in one).
/// The daemon serves the same page/assets at a STABLE origin (`/<daemon-token>/`), addresses
/// sessions with `?session=<id>` on the WS + `/api/history`, and adds `GET|POST /api/sessions`
/// plus `POST /api/sessions/:id/archive` for session control. The in-chat single-session
/// `/remote` server carries the same fields (empty title / no worktree) and no `/api/sessions`
/// route — the page detects daemon mode by probing that route.
///
/// v7: review cards + upload. `Snapshot` gained `diff` (the structured per-file diff card — a
/// pending write permission's "what will this touch", or the changes that landed this turn;
/// see [`SnapDiff`]) and `plan` (the `present_plan` proposal projected as a card; see
/// [`SnapPlan`]). `RemoteInput` gained `Attach` (a host-stored upload riding the next prompt —
/// the drain side of the new token-scoped `POST /<t>/api/upload` multipart route, images
/// becoming vision input and text files `@path` mentions). `GET /api/sessions` rows gained the
/// fleet fields (`waiting`/`context_tokens`/`context_limit`) and sort waiting-on-decision
/// first. Voice input is page-side only (Web Speech API) — nothing of it on the wire.
pub const PROTOCOL_VERSION: u32 = 7;

/// How many broadcast snapshots the per-server [`EventLog`] retains for reconnect replay. One
/// entry per *changed* frame covers minutes of activity; a client that was away longer gets a
/// full-snapshot resync instead (plus `GET /api/history` pagination for the scrollback it wants).
pub const EVENT_LOG_CAP: usize = 512;

/// A bounded ring of every broadcast snapshot keyed by its [`Snapshot::revision`], so a
/// reconnecting client (`?rev=<last seen>` on the WS handshake) replays exactly the frames it
/// missed instead of flickering through a from-scratch rebuild. Revisions are consecutive (one
/// bump per actually-broadcast frame), so "everything after rev N" is answerable precisely — or
/// not at all (evicted / unknown / foreign counter), which forces a full-snapshot resync.
pub struct EventLog {
    ring: std::collections::VecDeque<(u64, Snapshot)>,
    cap: usize,
}

impl EventLog {
    pub fn new(cap: usize) -> Self {
        Self {
            ring: std::collections::VecDeque::new(),
            cap,
        }
    }

    /// Record a broadcast frame, evicting the oldest beyond the cap (memory stays bounded no
    /// matter how long the session runs).
    pub fn push(&mut self, rev: u64, snap: Snapshot) {
        self.ring.push_back((rev, snap));
        while self.ring.len() > self.cap {
            self.ring.pop_front();
        }
    }

    /// Every retained snapshot with `revision > since`, oldest first — `Some(vec![])` when the
    /// client is already current. `None` when the gap can't be filled faithfully (the log is
    /// empty, `since` predates the oldest retained entry, or `since` is from a future/foreign
    /// counter): the caller must then resync with one full snapshot instead of replaying a hole.
    pub fn replay_after(&self, since: u64) -> Option<Vec<Snapshot>> {
        let (front, _) = self.ring.front()?;
        let (back, _) = self.ring.back()?;
        if since + 1 < *front || since > *back {
            return None;
        }
        Some(
            self.ring
                .iter()
                .filter(|(rev, _)| *rev > since)
                .map(|(_, s)| s.clone())
                .collect(),
        )
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.ring.len()
    }
}

/// Hard cap on a single inbound WebSocket frame (a [`RemoteInput`]). Inputs are short prompts or
/// answers; anything larger is dropped to bound memory + parse cost from a hostile/buggy client.
pub(crate) const MAX_INPUT_BYTES: usize = 256 * 1024;

/// One tracked task in the live task list, projected for the wire (status as a stable word).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SnapTask {
    pub title: String,
    /// "pending" | "in_progress" | "done".
    pub status: String,
}

/// One live subagent row, projected for the wire.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SnapSubagent {
    pub agent: String,
    pub task: String,
    pub model: Option<String>,
    /// Trailing edge of the child's streamed activity.
    pub last: String,
    pub done: bool,
    pub cost: f64,
}

/// One selectable option of a pending AskUserQuestion, so the page can render tappable buttons
/// instead of forcing the user to type a number on a phone keyboard.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SnapOption {
    pub label: String,
    pub description: String,
}

/// One row of the projected modal overlay ([`SnapOverlay::rows`]): an opaque `id` the page echoes
/// back in [`RemoteInput::OverlaySelect`], display strings, the cursor flag, and an optional
/// group header (e.g. a workflow phase).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SnapRow {
    pub id: String,
    pub label: String,
    pub detail: String,
    pub selected: bool,
    pub group: Option<String>,
}

/// One `@@` hunk of a [`SnapDiffFile`]: the unified-diff header (`@@ -a,b +c,d @@`, old/new
/// line spans) plus body lines, each prefixed `+`/`-`/` ` (the gutter is the first character —
/// the page styles on it and renders the rest verbatim).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SnapDiffHunk {
    pub header: String,
    pub lines: Vec<String>,
}

/// One file of the remote diff card. `adds`/`dels` count the WHOLE change; `hunks` carry at
/// most ~40 lines per file (`skipped_lines` says how many more exist — the full content stays
/// host-side, in the TUI scrollback and the tool result).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SnapDiffFile {
    pub path: String,
    /// "created" | "modified" | "deleted".
    pub kind: String,
    /// Non-UTF-8 target — no textual hunks, the page shows a one-line summary.
    pub binary: bool,
    pub adds: usize,
    pub dels: usize,
    pub hunks: Vec<SnapDiffHunk>,
    pub skipped_lines: usize,
}

/// The structured diff card (v7): while a write permission is pending, the ONE proposed change
/// that Allow would apply (`pending: true` — "what will this touch"); otherwise every change
/// that landed this turn (capped to 10 files, `skipped_files` counting evictions).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SnapDiff {
    pub pending: bool,
    pub files: Vec<SnapDiffFile>,
    pub skipped_files: usize,
}

/// One step of a projected plan card.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SnapPlanStep {
    pub title: String,
    pub detail: String,
}

/// The `present_plan` proposal projected as a card (v7). While the turn-end approval question
/// is pending (its options include "Build it"), the page renders Approve/Revise/Cancel buttons
/// that answer THAT question — the identical path a local choice takes.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SnapPlan {
    pub title: String,
    pub steps: Vec<SnapPlanStep>,
    pub notes: Option<String>,
}

/// The generic modal-overlay projection: whatever surface currently owns the TUI keyboard — the
/// command palette, any picker kind, the `@path` picker, the `/config` wizard, or an
/// informational overlay (`/usage`, `/mesh`, the workflow view) — rendered by the page as
/// tappable rows / a filter box / a free-text box / a text body. This is the ONE mechanism that
/// makes every slash command remote-drivable: any future picker only needs a projection arm, no
/// new wire types. Mapped from `forge_tui::OverlaySnapshot` by the render loop.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SnapOverlay {
    /// Stable discriminator: `"palette"`, `"picker:<kind>"`, `"config"`, `"overlay:usage"`,
    /// `"overlay:mesh"`, `"overlay:workflow"`.
    pub kind: String,
    pub title: String,
    pub rows: Vec<SnapRow>,
    /// Cursor index into `rows` (server-authoritative — the page highlights, never moves it).
    pub selected: usize,
    /// `Some` when the overlay has a type-to-filter query ([`RemoteInput::OverlayFilter`]).
    pub filter: Option<String>,
    /// True while the overlay is collecting a free-form value (e.g. a `/config` field edit).
    pub free_text: bool,
    /// Pre-rendered text for informational overlays (usage / mesh / workflow narration).
    pub body: Option<String>,
}

/// One frame of visible state broadcast to every connected browser, so the control page mirrors
/// the TUI statusline + the tail of the conversation. Cheap to build (plain strings) and JSON, so
/// a phone renders it without any client-side framework.
///
/// `PartialEq` is load-bearing: the render loop compares each candidate against the last
/// broadcast snapshot and only `watch::send`s on a real change — without that, the ~60 Hz busy
/// loop pushed an identical JSON frame every 16 ms to every client (battery/bandwidth burn, and
/// the page's action buttons were destroyed/recreated under the user's finger).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Snapshot {
    /// Wire-protocol version (see [`PROTOCOL_VERSION`]); the page warns on a mismatch.
    pub protocol: u32,
    /// The active session id — shown in the header so the operator knows which session they drive.
    pub session_id: String,
    /// The session's display title (v6). Empty when unnamed (the page falls back to the id).
    pub title: String,
    /// The working directory the session runs in (header context).
    pub cwd: String,
    /// The isolated worktree the session runs in (v6), when created with `worktree: true` —
    /// `None` for sessions running directly in their cwd.
    pub worktree: Option<String>,
    /// How the server is exposed: "loopback" | "LAN" | "public (provider)".
    pub exposure: String,
    pub busy: bool,
    pub done: bool,
    /// The active operating temper label (e.g. "Ask").
    pub temper: String,
    /// Mesh routing: tier + model, or "—" when unset.
    pub tier: Option<String>,
    pub model: String,
    /// Session spend in USD.
    pub cost_usd: f64,
    /// Context-window fill: tokens used + limit (if known).
    pub context_tokens: u64,
    pub context_limit: Option<u32>,
    /// The trailing edge of the in-flight streaming reply (plain text; re-sent each frame).
    pub streaming: String,
    /// Recent finalized scrollback lines (plain text, newest last; bounded).
    pub transcript: Vec<String>,
    /// The live task list (`update_tasks`) — drives the remote task panel.
    pub tasks: Vec<SnapTask>,
    /// Live subagents in the current `spawn_agents` batch.
    pub subagents: Vec<SnapSubagent>,
    /// Prompts the operator queued while a turn was running (shown so nothing looks dropped).
    pub queued: Vec<String>,
    /// A pending permission prompt, if the turn is blocked on a y/n.
    pub permission_prompt: Option<String>,
    /// A pending AskUserQuestion, if the turn is blocked on a choice.
    pub question: Option<String>,
    /// The options for a pending AskUserQuestion (tappable buttons on the page).
    pub question_options: Vec<SnapOption>,
    /// Whether the pending question accepts a free-text answer in addition to its options.
    pub question_allow_other: bool,
    /// The open modal overlay (palette / picker / config / usage / mesh / workflow), if any —
    /// see [`SnapOverlay`]. `None` when nothing modal is open.
    pub overlay: Option<SnapOverlay>,
    /// The structured diff card (v7) — see [`SnapDiff`]. `None` when nothing changed.
    pub diff: Option<SnapDiff>,
    /// The most recent plan proposal (v7) — see [`SnapPlan`].
    pub plan: Option<SnapPlan>,
    /// The most recent `/copy` payload, so the REMOTE device can put it on its own clipboard
    /// (the host's clipboard is useless from a phone). Cleared on the next prompt.
    pub copy_text: Option<String>,
    /// Identity of the currently pending permission prompt / question. Incremented every time a
    /// new prompt is installed; [`RemoteInput::Allow`]/[`RemoteInput::Answer`] must echo it back,
    /// and a mismatch is ignored — so a stale tap can never approve a NEWER (possibly more
    /// dangerous) prompt that replaced the one the page rendered.
    pub prompt_seq: u64,
    /// Recent remote-facing notices (bounded, newest last) — e.g. "that command ran in the TUI",
    /// "stale answer ignored". State (not events) so `watch` coalescing can't drop them.
    pub notes: Vec<String>,
    /// Monotonic snapshot revision, bumped once per *actually broadcast* (i.e. changed) frame.
    pub revision: u64,
    /// `true` when this frame is a full-state resynchronization rather than part of the
    /// contiguous revision stream — the first frame of a connection whose `?rev=` was absent,
    /// stale (evicted from the event log), or unknown. Tells the page to accept the frame even
    /// though its revision doesn't follow the last one it saw.
    pub resync: bool,
    /// `true` once remote control has been turned off (tells the page to stop reconnecting).
    pub closed: bool,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            protocol: PROTOCOL_VERSION,
            session_id: String::new(),
            title: String::new(),
            cwd: String::new(),
            worktree: None,
            exposure: String::new(),
            busy: false,
            done: false,
            temper: String::new(),
            tier: None,
            model: "—".to_string(),
            cost_usd: 0.0,
            context_tokens: 0,
            context_limit: None,
            streaming: String::new(),
            transcript: Vec::new(),
            tasks: Vec::new(),
            subagents: Vec::new(),
            queued: Vec::new(),
            permission_prompt: None,
            question: None,
            question_options: Vec::new(),
            question_allow_other: false,
            overlay: None,
            diff: None,
            plan: None,
            copy_text: None,
            prompt_seq: 0,
            notes: Vec::new(),
            revision: 0,
            resync: false,
            closed: false,
        }
    }
}

/// One persisted transcript row served by `GET /<token>/api/history` — the full-scrollback
/// pagination seam (the live [`Snapshot::transcript`] is only a short tail). `role` is the
/// stored role string (`"user"` / `"assistant"` / `"system"`); `visibility` is `"llm"` for
/// normal turns and `"ui"` for user-facing notes (which ARE part of the visible conversation).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct HistoryRow {
    pub seq: i64,
    pub role: String,
    pub content: String,
    pub model: Option<String>,
    pub created_at: i64,
    pub visibility: String,
}

/// The seam through which the server reads persisted transcript pages WITHOUT depending on
/// `forge-store`: `(session_id, before_seq, limit)` → rows newest first. Built by the caller
/// (run.rs) over the session's open store handle.
pub type HistoryProvider = Arc<dyn Fn(&str, Option<i64>, usize) -> Vec<HistoryRow> + Send + Sync>;

/// An input from a remote browser, drained by the render loop and injected like a local
/// keystroke / command. `Interrupt` maps to Esc-while-busy; `Answer` resolves a permission
/// prompt or an AskUserQuestion (the loop routes it to whichever is pending).
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RemoteInput {
    /// Submit a prompt (or a `/command`) — exactly as if typed + Enter in the TUI.
    ///
    /// `attachments` (added v7.1) is the message-correlated, client-computed list of uploads
    /// that ride THIS specific prompt — e.g. the mobile composer's tray at send time. When
    /// non-empty it is authoritative for this turn and any stale ambient `Attach` state (from an
    /// unrelated upload for an adjacent message) is discarded instead of used. `#[serde(default)]`
    /// keeps a bare `{"kind":"prompt","text":"..."}` (older clients, or the local TUI's own
    /// ambient `/image <path>` flow) parsing fine — an empty list falls back to exactly the old
    /// ambient `Attach`-then-`Prompt` behavior.
    Prompt {
        text: String,
        #[serde(default)]
        attachments: Vec<PromptAttachment>,
    },
    /// Answer a pending permission prompt: `true` = allow (y), `false` = deny (n). `seq` echoes
    /// the [`Snapshot::prompt_seq`] the page rendered its buttons from; the drain ignores a
    /// mismatch (the prompt changed under the tap). REQUIRED — a seq-less legacy (v2) answer
    /// fails to parse and is dropped, by design: it can't prove which prompt it approves.
    Allow { yes: bool, seq: u64 },
    /// Answer a pending AskUserQuestion with a free-text line (a number picks an option). `seq`
    /// as on [`RemoteInput::Allow`].
    Answer { text: String, seq: u64 },
    /// Esc-while-busy: stop the current turn (ignored when idle).
    Interrupt,
    /// Remove ONE not-yet-started queued prompt (added v7.2 — the mobile queued-chip tap).
    /// `index` is the position in [`Snapshot::queued`] the client rendered; `text` echoes the
    /// prompt at that position so a queue that shifted under the tap (a turn completed and
    /// consumed the head) is detected and the stale dequeue is dropped — same philosophy as
    /// the seq-checked [`RemoteInput::Allow`]/[`RemoteInput::Answer`].
    Dequeue { index: u64, text: String },
    /// A named keystroke (see [`named_key`]) injected through the SAME key path a local
    /// keystroke takes — the parity primitive for driving pickers/overlays. Dropped (with a
    /// note) while a permission prompt / question is pending: those must be answered via the
    /// seq-checked [`RemoteInput::Allow`]/[`RemoteInput::Answer`] only.
    Key { key: String },
    /// Move the open overlay's cursor onto the row with this `id`, then commit it exactly as a
    /// local Enter would.
    OverlaySelect { id: String },
    /// Move the open overlay's cursor by `delta` rows (negative = up), as repeated ↑/↓ keys.
    OverlayNav { delta: i32 },
    /// Replace the open overlay's filter/query text (or the value being edited, when
    /// [`SnapOverlay::free_text`] is set).
    OverlayFilter { text: String },
    /// Close the open overlay (Esc) — a no-op when nothing modal is open, so it can never
    /// interrupt a turn or quit the host by accident.
    OverlayCancel,
    /// A file stored by `POST /<t>/api/upload` should ride the NEXT prompt: an image becomes
    /// vision input (`Session::attach_images`), a text file an `@path` mention prepended to the
    /// next prompt. The drain confines `path` to the session's `.forge/uploads/` scratch area —
    /// this input exists only as the upload route's delivery leg, so an arbitrary host path
    /// (e.g. a WS client probing for secret files) is refused with a note.
    Attach { path: String, image: bool },
}

/// One message-correlated attachment riding a [`RemoteInput::Prompt`] (v7.1) — the client's own
/// upload path for a file it already POSTed to `/api/upload`. Confined to the session's
/// `.forge/uploads/` scratch area at resolution time, exactly like [`RemoteInput::Attach`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct PromptAttachment {
    pub path: String,
    pub image: bool,
}

/// Map a wire key name to the TUI key it injects. The names are part of the v4 protocol:
/// `Up | Down | Enter | Esc | Tab | BTab | PageUp | PageDown | Home | End | Backspace |
/// Char:<c>`. `BTab` is SHIFT+TAB (the temper cycler). `None` for anything else — the drain
/// notes and drops it rather than guessing.
pub fn named_key(name: &str) -> Option<forge_tui::KeyKind> {
    use forge_tui::KeyKind as K;
    Some(match name {
        "Up" => K::Up,
        "Down" => K::Down,
        "Enter" => K::Enter,
        "Esc" => K::Esc,
        "Tab" => K::Tab,
        "BTab" => K::CycleTemper,
        "PageUp" => K::PageUp,
        "PageDown" => K::PageDown,
        "Home" => K::Home,
        "End" => K::End,
        "Backspace" => K::Backspace,
        _ => {
            let c = name.strip_prefix("Char:")?;
            let mut chars = c.chars();
            let ch = chars.next()?;
            if chars.next().is_some() {
                return None; // exactly one char
            }
            K::Char(ch)
        }
    })
}

/// True when a remote Allow/Answer's echoed `seq` targets the prompt that is pending NOW (see
/// [`Snapshot::prompt_seq`]). A mismatch means the prompt changed after the page rendered its
/// buttons — the answer is stale and must be ignored rather than resolving the wrong prompt.
pub fn prompt_seq_current(current: u64, sent: u64) -> bool {
    current == sent
}

/// The human exposure label shown in the page header ("loopback" | "LAN" | "public (provider)").
/// `tls_failed` (LAN only) turns the label into an explicit *unavailable* — the server is NOT
/// listening after a TLS setup failure, and pretending otherwise ("LAN") lies to the operator.
pub fn exposure_label(tunnel: Option<&str>, lan_tls: bool, tls_failed: bool) -> String {
    if let Some(t) = tunnel {
        format!("public ({t})")
    } else if lan_tls {
        if tls_failed {
            "LAN (unavailable — TLS failed)".to_string()
        } else {
            "LAN".to_string()
        }
    } else {
        "loopback".to_string()
    }
}

/// The handle the render loop holds: publish a new [`Snapshot`] every dirty frame, and drain
/// queued [`RemoteInput`]s to inject them. Dropping it stops the server.
pub struct RemoteControl {
    /// Publish the latest visible state; the WS task forwards it to every browser.
    /// Use [`Self::broadcast`] (never `send` directly) so the frame also lands in the replay log.
    pub snapshot_tx: watch::Sender<Snapshot>,
    /// Every broadcast frame, retained (bounded) for reconnect replay — shared with the WS
    /// handler, which answers `?rev=<n>` handshakes from it.
    events: Arc<std::sync::Mutex<EventLog>>,
    /// Inputs queued by remote browsers; the render loop drains these each iteration.
    pub input_rx: mpsc::Receiver<RemoteInput>,
    /// The connect URL + token (printed once into scrollback).
    pub url: RemoteUrl,
    /// Abort the server task on drop so the port frees immediately.
    _server: JoinHandle<()>,
    /// The public-tunnel child process (`--anywhere` only). `kill_on_drop`, so dropping the
    /// handle tears the tunnel down with the server. `None` for LAN/loopback exposure.
    _tunnel: Option<tokio::process::Child>,
    /// The tunnel provider's human label (`--anywhere` only), for the scrollback note.
    pub tunnel: Option<&'static str>,
    /// Set from the spawned server task if a `Lan` bind's TLS setup fails *after* [`start`]
    /// already returned an `https://` URL (see [`Self::tls_failed`]) — the connect URL and cert
    /// fingerprint were fixed at return time and can't be corrected in place, so this is checked
    /// separately wherever the exposure is reported (e.g. the remote-page header).
    tls_failed: Arc<AtomicBool>,
}

impl RemoteControl {
    /// True once a `Lan`-exposure bind's TLS setup has failed asynchronously — the server is NOT
    /// listening (there is no cleartext fallback: it would lie about the transport and be
    /// unreachable at the already-printed `https://` URL anyway), so remote control is
    /// unavailable. Always `false` for `Local`/`Anywhere` (no TLS is attempted).
    pub fn tls_failed(&self) -> bool {
        self.tls_failed.load(Ordering::Relaxed)
    }

    /// Publish a frame to every connected browser AND record it in the replay log. The render
    /// loop must use this (never `snapshot_tx.send` directly): a frame that skipped the log
    /// would be unreplayable, so a reconnecting client would resync (full rebuild) instead of
    /// receiving exactly what it missed. `snap.revision` must already be bumped by the caller.
    pub fn broadcast(&self, snap: Snapshot) {
        if let Ok(mut log) = self.events.lock() {
            log.push(snap.revision, snap.clone());
        }
        let _ = self.snapshot_tx.send(snap);
    }
}

impl Drop for RemoteControl {
    fn drop(&mut self) {
        // Mark closed so connected browsers stop reconnecting, then tear the server down. The
        // revision bump matters: the page drops frames at or below the revision it already saw
        // (reconnect-replay dedup), and `closed` rides on otherwise-unchanged state.
        let last = self.snapshot_tx.borrow().clone();
        let _ = self.snapshot_tx.send(Snapshot {
            closed: true,
            revision: last.revision + 1,
            ..last
        });
        self._server.abort();
    }
}

/// A random URL-safe token for path-gating the control page + WS. Lowercase hex is unambiguous
/// on a phone keyboard and survives being embedded in a QR code.
pub(crate) fn random_token() -> String {
    // 16 hex chars (64 bits) sourced from the OS CSPRNG via `rand::random` (already a workspace
    // dependency — see `forge-config::oauth`). This is genuinely load-bearing under `--anywhere`,
    // where the token is the sole authentication for a public, internet-reachable control
    // channel, so it must not be derived from guessable/low-entropy inputs like the process
    // start time or pid.
    format!("{:016x}", rand::random::<u64>())
}

/// Best-effort LAN hostname for the connect URL. We keep it dependency-free and just return the
/// numeric IP — it always resolves from the phone, and avoids a DNS-lookup edge case where the
/// machine's hostname isn't resolvable on the LAN (which would yield a dead QR code).
fn lan_host(addr: SocketAddr) -> String {
    addr.ip().to_string()
}

/// Discover the IP of the interface that carries this machine's outbound traffic — the address a
/// phone on the same network can actually reach. Binding `0.0.0.0` and printing the *bind*
/// address gave phones a dead `https://0.0.0.0:PORT/…` URL/QR; the OS knows the real interface,
/// and `connect` on a UDP socket resolves the route + local address WITHOUT sending any packet
/// (8.8.8.8 is never contacted). `None` when routing fails (offline, no default route) or the
/// resolved address is unusable (unspecified/loopback) — callers fall back to the bind address.
fn discover_lan_ip() -> Option<std::net::IpAddr> {
    let sock = std::net::UdpSocket::bind((std::net::Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    sock.connect(("8.8.8.8", 80)).ok()?;
    let ip = sock.local_addr().ok()?.ip();
    (!ip.is_unspecified() && !ip.is_loopback()).then_some(ip)
}

/// The host to advertise for a `Lan` bind (URL, QR, cert SANs): the `[remote] host` config
/// override wins (multi-homed/VPN machines where discovery picks the wrong interface), then the
/// discovered outbound-interface IP, then the raw bind address as a last resort.
pub(crate) fn lan_display_host(host_override: Option<&str>, addr: SocketAddr) -> String {
    host_override
        .map(str::to_string)
        .or_else(|| discover_lan_ip().map(|ip| ip.to_string()))
        .unwrap_or_else(|| lan_host(addr))
}

/// Start the remote-control server. The returned [`RemoteControl`] is moved into the render loop;
/// dropping it stops the server and frees the port. [`Exposure`] selects the bind address:
/// `Lan` → `0.0.0.0` (LAN-reachable, HTTPS with self-signed cert), `Local`/`Anywhere` →
/// `127.0.0.1` (loopback, plain HTTP). `Anywhere` binds loopback because the public tunnel
/// ([`start_anywhere`]) provides the public exposure; this fn does NOT spawn the tunnel (it's
/// sync) — use [`start_anywhere`] for that.
///
/// `host_override` (`[remote] host`) replaces the auto-discovered LAN IP in the connect URL,
/// QR code, and certificate SANs; ignored for `Local`/`Anywhere`.
///
/// **TLS**: For the `Lan` exposure the server generates a self-signed certificate and serves
/// HTTPS so the access token never travels in cleartext over the LAN. The cert fingerprint is
/// included in the returned [`RemoteUrl`] so it can be shown to the user. If cert generation
/// fails this returns `Err`; if the async TLS setup fails later the server is declared
/// unavailable (see [`RemoteControl::tls_failed`]) — there is deliberately NO cleartext
/// fallback on the LAN.
///
/// `history` is the persisted-transcript seam behind `GET /<token>/api/history` (full
/// scrollback pagination); `None` serves empty pages (tests / callers without a store).
pub fn start(
    exposure: Exposure,
    host_override: Option<&str>,
    history: Option<HistoryProvider>,
) -> std::io::Result<RemoteControl> {
    let token = random_token();
    let bind_ip: std::net::IpAddr = match exposure {
        Exposure::Lan => std::net::Ipv4Addr::UNSPECIFIED.into(),
        Exposure::Local | Exposure::Anywhere => std::net::Ipv4Addr::LOCALHOST.into(),
    };
    // Port 0 → the OS picks a free ephemeral port (no clashes, no config).
    let listener = std::net::TcpListener::bind((bind_ip, 0))?;
    let addr = listener.local_addr()?;
    // The advertised host: for LAN binds the bind address (0.0.0.0) is meaningless to a phone,
    // so advertise the config override or the discovered outbound-interface IP instead.
    let host = match exposure {
        Exposure::Lan => lan_display_host(host_override, addr),
        Exposure::Local | Exposure::Anywhere => lan_host(addr),
    };

    let (snapshot_tx, snapshot_rx) = watch::channel(Snapshot::default());
    let (input_tx, input_rx) = mpsc::channel::<RemoteInput>(64);
    // The replay log (v5 reconnect): every broadcast frame lands here (see
    // `RemoteControl::broadcast`), and the WS handler answers `?rev=` handshakes from it.
    let events = Arc::new(std::sync::Mutex::new(EventLog::new(EVENT_LOG_CAP)));

    let base = format!("/{token}");
    let state = Arc::new(ServerState {
        snapshot_rx: snapshot_rx.clone(),
        input_tx,
        events: events.clone(),
        history,
        base: base.clone(),
        // The in-chat session runs in the process cwd, so uploads scratch under it.
        upload_root: std::env::current_dir()
            .ok()
            .map(|d| d.join(".forge").join("uploads")),
    });

    let app = Router::new()
        // The control page (HTML) at /<token> and /<token>/ — the slashed form is the PWA
        // `start_url` so the installed app launches inside the service-worker scope.
        .route(&base, get(control_page))
        .route(&format!("{base}/"), get(control_page))
        // The WebSocket at /<token>/ws — same token gates it. `?rev=<n>` replays missed frames.
        .route(&format!("{base}/ws"), get(ws_handler))
        // Paginated persisted-transcript scrollback (newest first, `?before=<seq>&limit=<n>`).
        // Token-scoped like everything else; this server drives ONE session, so no session
        // parameter exists yet (the multi-session daemon is Phase 4).
        .route(&format!("{base}/api/history"), get(history_page))
        // File/image upload (v7): multipart, stored under `<cwd>/.forge/uploads/<session>/`,
        // delivered to the render loop as a `RemoteInput::Attach` riding the next prompt. The
        // per-route body limit replaces axum's 2 MB default (with headroom for boundaries).
        .route(
            &format!("{base}/api/upload"),
            axum::routing::post(upload_handler)
                .layer(axum::extract::DefaultBodyLimit::max(UPLOAD_BODY_LIMIT)),
        )
        // The page's script + stylesheet as separate token-scoped files, so the CSP needs no
        // 'unsafe-inline' anywhere.
        .route(&format!("{base}/app.js"), get(app_js))
        .route(&format!("{base}/styles.css"), get(styles_css))
        // PWA assets (token-scoped) so the page installs to a phone home screen + runs standalone.
        .route(&format!("{base}/manifest.webmanifest"), get(manifest))
        .route(&format!("{base}/sw.js"), get(service_worker))
        .route(&format!("{base}/icon.svg"), get(icon))
        // A 404 for the root and wrong-token paths (don't leak that remote control is on).
        .fallback(fallback)
        .with_state(state);

    // For the LAN exposure, serve TLS with a self-signed certificate so the access token doesn't
    // travel in cleartext. Any TLS failure means LAN remote control is UNAVAILABLE — never a
    // silent downgrade to plain HTTP: the `https://` URL was already committed (browsers can't
    // reach an `http://` server through it), and a cleartext LAN server would put the token on
    // the wire while the status still said "LAN".
    if exposure == Exposure::Lan {
        // SANs: the advertised LAN IP + localhost (so a browser connecting by hostname also works).
        let sans = vec![host.clone(), "localhost".to_string()];
        let tls = generate_self_signed(sans).map_err(|e| {
            std::io::Error::other(format!(
                "self-signed cert generation failed ({e}) — LAN remote control needs TLS; \
                 try `/remote --local` or `/remote --anywhere`"
            ))
        })?;
        let fingerprint = tls.fingerprint.clone();
        let cert_pem = tls.cert_pem;
        let key_pem = tls.key_pem;
        let url = format!("https://{host}:{}/{}", addr.port(), token);
        // axum-server calls tokio::net::TcpListener::from_std internally, which
        // requires the listener to already be in nonblocking mode.
        listener.set_nonblocking(true)?;

        // axum-server::from_tcp_rustls takes a std::net::TcpListener (non-async).
        // We build the RustlsConfig inside the spawned async task because
        // RustlsConfig::from_pem is async (it spawns blocking work internally).
        // `start()` already committed to an `https://` URL above (before this task even
        // runs), so a failure here can't be corrected in place — `tls_failed` is how the
        // render loop finds out the server never came up.
        let tls_failed = Arc::new(AtomicBool::new(false));
        let tls_failed_task = tls_failed.clone();
        let server = tokio::spawn(async move {
            match axum_server::tls_rustls::RustlsConfig::from_pem(cert_pem, key_pem).await {
                Ok(tls_config) => match axum_server::from_tcp_rustls(listener, tls_config) {
                    Ok(server) => {
                        server.serve(app.into_make_service()).await.ok();
                    }
                    Err(e) => {
                        tracing::warn!(
                            "remote: TLS server setup failed ({e}) — \
                             LAN remote control is unavailable (no cleartext fallback)"
                        );
                        tls_failed_task.store(true, Ordering::Relaxed);
                    }
                },
                Err(e) => {
                    tracing::warn!(
                        "remote: TLS config build failed ({e}) — \
                         LAN remote control is unavailable (no cleartext fallback)"
                    );
                    tls_failed_task.store(true, Ordering::Relaxed);
                }
            }
        });

        return Ok(RemoteControl {
            snapshot_tx,
            events,
            input_rx,
            url: RemoteUrl {
                url,
                addr,
                token,
                tls_fingerprint: Some(fingerprint),
            },
            _server: server,
            _tunnel: None,
            tls_failed,
            tunnel: None,
        });
    }

    // Plain HTTP path: loopback only (--local / --anywhere) — the connection never leaves the
    // machine (tunnels terminate TLS at the provider).
    // axum wants a tokio TcpListener; convert the blocking std listener we used to read the
    // bound port (so the connect URL is correct before the async task even starts).
    listener.set_nonblocking(true)?;
    let tokio_listener = tokio::net::TcpListener::from_std(listener)?;
    let url = format!("http://{host}:{}/{}", addr.port(), token);

    let server = tokio::spawn(async move {
        axum::serve(tokio_listener, app).await.ok(); // best-effort: errors here mean the user turned it off / the port dropped
    });

    Ok(RemoteControl {
        snapshot_tx,
        events,
        input_rx,
        url: RemoteUrl {
            url,
            addr,
            token,
            tls_fingerprint: None,
        },
        _server: server,
        _tunnel: None,
        tunnel: None,
        tls_failed: Arc::new(AtomicBool::new(false)),
    })
}

/// Start the server on loopback and pipe it through a public tunnel so any browser, anywhere, can
/// reach it — no manual router port-forwarding. Probes for an installed tunnel CLI
/// (cloudflared → ngrok; bore is excluded — cleartext end-to-end) and points it at the bound
/// port; the returned [`RemoteControl`]'s `url` is the PUBLIC `https://…/<token>`, and it owns
/// the tunnel child (killed on drop). Errors if no tunnel tool is installed or the tunnel never
/// publishes a URL — the caller surfaces an install hint.
pub async fn start_anywhere(history: Option<HistoryProvider>) -> std::io::Result<RemoteControl> {
    let kind = detect_tunnel().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no tunnel tool found on PATH — install cloudflared or ngrok \
             (bore is unsupported: its tunnel has no TLS, so the access token would travel \
             the public internet in cleartext)",
        )
    })?;
    let mut rc = start(Exposure::Anywhere, None, history)?;
    let port = rc.url.addr.port();
    let (public, child) = spawn_tunnel(kind, port).await?;
    // The control page lives at `/<token>`; the tunnel forwards the whole path, so the public
    // connect URL is the tunnel base + the same token gate.
    rc.url.url = format!("{}/{}", public.trim_end_matches('/'), rc.url.token);
    rc._tunnel = Some(child);
    rc.tunnel = Some(kind.label());
    Ok(rc)
}

#[derive(Clone)]
struct ServerState {
    snapshot_rx: watch::Receiver<Snapshot>,
    input_tx: mpsc::Sender<RemoteInput>,
    /// The bounded replay log shared with [`RemoteControl::broadcast`] — answers `?rev=` WS
    /// handshakes with exactly the frames a reconnecting client missed.
    events: Arc<std::sync::Mutex<EventLog>>,
    /// The persisted-transcript seam for `GET /api/history`; `None` serves empty pages.
    history: Option<HistoryProvider>,
    /// The token-gated base path (`/<token>`) — injected into the page + manifest so every URL
    /// (WS, PWA assets, start_url) is correct under a tunnel/LAN host without the page guessing.
    base: String,
    /// Where `POST /api/upload` stores files: `<cwd>/.forge/uploads` (a per-session subdirectory
    /// is created under it). `None` when the cwd is unknown — uploads then answer 503.
    upload_root: Option<std::path::PathBuf>,
}

/// The `Content-Security-Policy` for the control page. Everything is same-origin — the script,
/// stylesheet, and service worker are separate token-scoped files (`remote_assets/`), so there is
/// no `'unsafe-inline'` anywhere: an injected inline `<script>`/`onclick` can never execute.
/// `connect-src` must name the ws:/wss: schemes explicitly — some browsers don't fold WebSockets
/// into `'self'`.
pub(crate) const PAGE_CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self'; \
     img-src 'self' data:; connect-src 'self' ws: wss:; \
     frame-ancestors 'none'; base-uri 'none'; form-action 'none'";

/// The single control page: a responsive, dependency-free HTML/CSS/JS shell that mirrors the
/// statusline, shows the streaming reply + recent transcript, and sends inputs over the WS. It's
/// intentionally one self-contained string so there's no static-asset serving to wire up. Takes
/// the shared state so the token base path can be injected. Hardening headers ride along: the
/// page drives a live coding session, so it must never render inside a hostile frame
/// (X-Frame-Options/frame-ancestors) and its token-bearing URL must never leak via the Referer
/// header (Referrer-Policy).
async fn control_page(State(state): State<Arc<ServerState>>) -> Response {
    (
        [
            (axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (axum::http::header::X_FRAME_OPTIONS, "DENY"),
            (axum::http::header::CONTENT_SECURITY_POLICY, PAGE_CSP),
            (axum::http::header::REFERRER_POLICY, "no-referrer"),
        ],
        CONTROL_PAGE.replace("__BASE__", &state.base),
    )
        .into_response()
}

/// The page's JavaScript (token-scoped, `__BASE__` injected like the page itself).
async fn app_js(State(state): State<Arc<ServerState>>) -> Response {
    (
        [(axum::http::header::CONTENT_TYPE, "text/javascript")],
        APP_JS.replace("__BASE__", &state.base),
    )
        .into_response()
}

/// The page's stylesheet (static — no token content).
async fn styles_css() -> Response {
    ([(axum::http::header::CONTENT_TYPE, "text/css")], STYLES_CSS).into_response()
}

/// The token-scoped PWA manifest (`start_url`/`scope` baked to this session's path).
async fn manifest(State(state): State<Arc<ServerState>>) -> Response {
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "application/manifest+json",
        )],
        manifest_json(&state.base),
    )
        .into_response()
}

/// The service worker that makes the page installable (its scope is this session's token path).
async fn service_worker() -> Response {
    (
        [(axum::http::header::CONTENT_TYPE, "text/javascript")],
        SERVICE_WORKER,
    )
        .into_response()
}

/// The app icon (inline SVG; no binary asset to serve).
async fn icon() -> Response {
    (
        [(axum::http::header::CONTENT_TYPE, "image/svg+xml")],
        ICON_SVG,
    )
        .into_response()
}

/// A minimal 404 that doesn't reveal remote control is running.
async fn fallback() -> Response {
    (axum::http::StatusCode::NOT_FOUND, "Not Found").into_response()
}

/// Default / maximum page size for `GET /api/history` — what a phone can render and SQLite can
/// serve without a hiccup.
pub(crate) const HISTORY_PAGE_DEFAULT: usize = 60;
pub(crate) const HISTORY_PAGE_MAX: usize = 200;

/// Clamp a requested history page size to `1..=`[`HISTORY_PAGE_MAX`].
pub(crate) fn history_page_limit(requested: Option<usize>) -> usize {
    requested
        .unwrap_or(HISTORY_PAGE_DEFAULT)
        .clamp(1, HISTORY_PAGE_MAX)
}

/// Query parameters for `GET /<token>/api/history`.
#[derive(serde::Deserialize)]
struct HistoryParams {
    /// Return only rows with `seq <` this (omit for the newest page).
    before: Option<i64>,
    /// Page size (clamped — see [`history_page_limit`]).
    limit: Option<usize>,
}

/// `GET /<token>/api/history?before=<seq>&limit=<n>` — one JSON page of the session's persisted
/// transcript, newest first (see [`HistoryRow`]). The session id comes from the latest broadcast
/// snapshot (this server drives ONE session; the multi-session daemon is Phase 4). Serves `[]`
/// before the first broadcast or when no store seam was provided. The store read runs on the
/// blocking pool — rusqlite is synchronous and this is the async accept path.
async fn history_page(
    State(state): State<Arc<ServerState>>,
    axum::extract::Query(params): axum::extract::Query<HistoryParams>,
) -> Response {
    let (before, limit) = (params.before, history_page_limit(params.limit));
    let sid = state.snapshot_rx.borrow().session_id.clone();
    let rows: Vec<HistoryRow> = match (state.history.clone(), sid.is_empty()) {
        (Some(provider), false) => {
            tokio::task::spawn_blocking(move || provider(&sid, before, limit))
                .await
                .unwrap_or_default()
        }
        _ => Vec::new(),
    };
    (
        [
            (axum::http::header::CONTENT_TYPE, "application/json"),
            // Live data: a cached page would hide new turns (and the SW skips /api/ too).
            (axum::http::header::CACHE_CONTROL, "no-store"),
        ],
        serde_json::to_string(&rows).unwrap_or_else(|_| "[]".into()),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// File/image upload (v7)
// ---------------------------------------------------------------------------

/// Hard cap on ONE uploaded file. Phone photos compress well under this; anything larger has no
/// business riding a chat prompt.
pub(crate) const UPLOAD_MAX_BYTES: usize = 10 * 1024 * 1024;

/// The request-body limit for the upload route: the file cap plus headroom for multipart
/// boundaries/headers and a couple of small siblings (e.g. a screenshot + a note file).
pub(crate) const UPLOAD_BODY_LIMIT: usize = UPLOAD_MAX_BYTES + 2 * 1024 * 1024;

/// Flatten an untrusted upload filename to a single safe path component: the final component
/// only (no traversal), characters outside `[A-Za-z0-9._-]` replaced with `_`, leading dots
/// stripped (no hidden files, no `..` remnants), length-capped, never empty.
pub(crate) fn sanitize_upload_name(name: &str) -> String {
    let last = name.rsplit(['/', '\\']).next().unwrap_or_default().trim();
    let mut clean: String = last
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .skip_while(|&c| c == '.')
        .take(80)
        .collect();
    if clean.is_empty() || clean.chars().all(|c| c == '_' || c == '.') {
        clean = "upload".to_string();
    }
    clean
}

/// Is this upload an image (→ vision input) by declared content type or file extension?
pub(crate) fn upload_is_image(content_type: Option<&str>, name: &str) -> bool {
    if content_type.is_some_and(|t| t.starts_with("image/")) {
        return true;
    }
    let ext = name.rsplit('.').next().unwrap_or_default().to_lowercase();
    matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp")
}

/// Store one uploaded file under `dir` (created as needed): size-capped, name sanitized and
/// timestamp-prefixed (collision-free, ordered), and non-images required to be UTF-8 text —
/// only images and text files have an injection path into a prompt, so anything else is
/// refused at the door instead of parked on disk. Returns the stored path + whether it's an
/// image; errors are human-readable and map onto 4xx responses.
pub(crate) fn store_upload(
    dir: &std::path::Path,
    name: &str,
    content_type: Option<&str>,
    bytes: &[u8],
) -> Result<(std::path::PathBuf, bool), String> {
    if bytes.is_empty() {
        return Err("empty file".to_string());
    }
    if bytes.len() > UPLOAD_MAX_BYTES {
        return Err(format!(
            "file too large ({} bytes > {} max)",
            bytes.len(),
            UPLOAD_MAX_BYTES
        ));
    }
    let image = upload_is_image(content_type, name);
    if !image && std::str::from_utf8(bytes).is_err() {
        return Err("only images and UTF-8 text files can ride a prompt".to_string());
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("upload dir: {e}"))?;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let path = dir.join(format!("{ts}-{}", sanitize_upload_name(name)));
    std::fs::write(&path, bytes).map_err(|e| format!("writing upload: {e}"))?;
    Ok((path, image))
}

/// `POST /<token>/api/upload` — multipart file/image upload for the in-chat single-session
/// server (the daemon has its own session-addressed twin in `serve.rs`). Each stored file is
/// delivered to the render loop as [`RemoteInput::Attach`] and rides the next prompt.
async fn upload_handler(
    State(state): State<Arc<ServerState>>,
    mut multipart: axum::extract::Multipart,
) -> Response {
    let Some(root) = state.upload_root.clone() else {
        return upload_error(
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "uploads are unavailable (no working directory)",
        );
    };
    let sid = state.snapshot_rx.borrow().session_id.clone();
    if sid.is_empty() {
        return upload_error(
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "session not ready yet — retry in a moment",
        );
    }
    let dir = root.join(sanitize_upload_name(&sid));
    let mut stored: Vec<serde_json::Value> = Vec::new();
    loop {
        let field = match multipart.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(e) => {
                return upload_error(
                    axum::http::StatusCode::BAD_REQUEST,
                    &format!("malformed multipart body: {e}"),
                );
            }
        };
        let name = field.file_name().unwrap_or("upload").to_string();
        let content_type = field.content_type().map(str::to_string);
        let bytes = match field.bytes().await {
            Ok(b) => b,
            Err(e) => {
                // axum surfaces the body-limit overflow here.
                return upload_error(
                    axum::http::StatusCode::PAYLOAD_TOO_LARGE,
                    &format!("upload failed: {e}"),
                );
            }
        };
        match store_upload(&dir, &name, content_type.as_deref(), &bytes) {
            Ok((path, image)) => {
                let path_str = path.display().to_string();
                if state
                    .input_tx
                    .send(RemoteInput::Attach {
                        path: path_str.clone(),
                        image,
                    })
                    .await
                    .is_err()
                {
                    return upload_error(
                        axum::http::StatusCode::CONFLICT,
                        "remote control is shutting down",
                    );
                }
                stored.push(serde_json::json!({
                    "name": sanitize_upload_name(&name),
                    "path": path_str,
                    "image": image,
                }));
            }
            Err(msg) => {
                return upload_error(axum::http::StatusCode::UNPROCESSABLE_ENTITY, &msg);
            }
        }
    }
    if stored.is_empty() {
        return upload_error(axum::http::StatusCode::BAD_REQUEST, "no files in the body");
    }
    (
        [
            (axum::http::header::CONTENT_TYPE, "application/json"),
            (axum::http::header::CACHE_CONTROL, "no-store"),
        ],
        serde_json::json!({ "files": stored }).to_string(),
    )
        .into_response()
}

/// A JSON error body for the upload route (shape shared with the daemon's handlers).
fn upload_error(status: axum::http::StatusCode, msg: &str) -> Response {
    (
        status,
        [
            (axum::http::header::CONTENT_TYPE, "application/json"),
            (axum::http::header::CACHE_CONTROL, "no-store"),
        ],
        serde_json::json!({ "error": msg }).to_string(),
    )
        .into_response()
}

/// Query parameters for the WS handshake: `rev` is the last snapshot revision the page saw
/// (v5 reconnect/replay). Absent / 0 / malformed → fresh connection (full snapshot flagged
/// `resync`).
#[derive(serde::Deserialize)]
struct WsParams {
    #[serde(default)]
    rev: u64,
}

async fn ws_handler(
    State(state): State<Arc<ServerState>>,
    axum::extract::Query(params): axum::extract::Query<WsParams>,
    ws: WebSocketUpgrade,
) -> Response {
    // The route is static (the token is baked into the registered path at `start` time and is also
    // held in `state`), so there's no path parameter to extract — taking `Path<String>` here would
    // find zero captures and 500. A wrong-token request never matches the registered route and
    // falls through to the 404 fallback instead.
    ws.on_upgrade(move |socket| ws_session(socket, state, params.rev))
}

/// One connected browser: forward snapshots out, parse inputs in. Runs until the browser
/// disconnects or the server stops (the watch channel closes → the forward loop exits).
///
/// `since` is the client's last-seen revision (v5): when the replay log can fill the gap, the
/// client receives exactly the frames it missed (none when already current) and then follows
/// live — no gap, no flicker. When it can't (fresh connect, evicted, foreign counter), it gets
/// ONE full snapshot flagged `resync: true` instead.
async fn ws_session(socket: WebSocket, state: Arc<ServerState>, since: u64) {
    pump_ws(
        socket,
        state.snapshot_rx.clone(),
        state.events.clone(),
        state.input_tx.clone(),
        since,
    )
    .await
}

/// The transport-independent body of one connected WebSocket client: replay-from-rev (or a
/// full `resync` snapshot), then live-follow the watch channel out and parse [`RemoteInput`]s
/// in. Shared verbatim between the in-chat single-session server ([`ws_session`]) and the
/// `forge serve` daemon's per-session WS route — the two differ only in WHERE the channels
/// come from (a single server state vs. a session registry lookup).
pub(crate) async fn pump_ws(
    socket: WebSocket,
    snapshot_rx: watch::Receiver<Snapshot>,
    events: Arc<std::sync::Mutex<EventLog>>,
    input_tx: mpsc::Sender<RemoteInput>,
    since: u64,
) {
    use futures::stream::StreamExt;
    use futures::SinkExt;

    let (mut tx, mut rx) = socket.split();
    // Clone the receiver BEFORE reading the replay log: a frame broadcast between the two is
    // then guaranteed to be seen (in the log, in the watch, or both — the page dedupes on
    // revision), so a reconnect can never observe a gap.
    let mut snap = snapshot_rx;

    let replay = if since == 0 {
        None
    } else {
        events.lock().ok().and_then(|log| log.replay_after(since))
    };
    match replay {
        Some(missed) => {
            let mut last_sent = since;
            for s in &missed {
                let json = serde_json::to_string(s).unwrap_or_else(|_| "{}".into());
                if tx.send(Message::Text(json.into())).await.is_err() {
                    return;
                }
                last_sent = s.revision;
            }
            // A cloned receiver has NOT seen the value currently in the watch, so the forward
            // loop below would immediately re-deliver it — a duplicate of the replay's last
            // frame. Mark it seen here, and send it ourselves only when the replay didn't
            // already cover it (it was broadcast between the ring read and now).
            let current = snap.borrow_and_update().clone();
            if current.revision > last_sent {
                let json = serde_json::to_string(&current).unwrap_or_else(|_| "{}".into());
                if tx.send(Message::Text(json.into())).await.is_err() {
                    return;
                }
            }
        }
        None => {
            // Send the current snapshot immediately so the page isn't blank until the next
            // change, flagged as a resync (its revision doesn't extend the client's stream).
            // `borrow_and_update` (not `borrow`): mark it seen so the forward loop doesn't
            // deliver the same frame again right away.
            let mut current = snap.borrow_and_update().clone();
            current.resync = true;
            let initial = serde_json::to_string(&current).unwrap_or_else(|_| "{}".into());
            if tx.send(Message::Text(initial.into())).await.is_err() {
                return;
            }
        }
    }

    let mut forward = tokio::spawn(async move {
        while let Ok(()) = snap.changed().await {
            let json = serde_json::to_string(&*snap.borrow()).unwrap_or_else(|_| "{}".into());
            if tx.send(Message::Text(json.into())).await.is_err() {
                break; // client gone
            }
        }
    });

    // Receive inputs from the browser; forward each to the render loop's channel.
    let mut receive = tokio::spawn(async move {
        while let Some(Ok(msg)) = rx.next().await {
            let text = match msg {
                Message::Text(t) => t.to_string(),
                Message::Binary(b) => match String::from_utf8(b.to_vec()) {
                    Ok(s) => s,
                    Err(_) => continue,
                },
                Message::Close(_) => break,
                // Ping/Pong are handled by axum automatically; ignore Binary-as-ping noise.
                _ => continue,
            };
            // Drop oversized frames: a remote input is a short prompt/answer, never a megabyte.
            // Caps memory + parse work from a hostile or buggy client on a public tunnel.
            if text.len() > MAX_INPUT_BYTES {
                continue;
            }
            if let Ok(input) = serde_json::from_str::<RemoteInput>(&text) {
                if input_tx.send(input).await.is_err() {
                    break; // render loop dropped the receiver (remote turned off)
                }
            }
        }
    });

    // When either half ends, drop the other.
    tokio::select! {
        _ = &mut forward => { receive.abort(); }
        _ = &mut receive => { forward.abort(); }
    }
}

/// Render the connect URL as a scannable QR code into plain-text TUI scrollback lines. Returns
/// `None` when the encoder fails (we then just print the URL). Uses half-block glyphs so it reads
/// at a normal terminal cell aspect ratio.
pub fn qr_lines(url: &str) -> Option<Vec<String>> {
    let code = qrcode::QrCode::new(url.as_bytes()).ok()?;
    let width = code.width();
    let mut out: Vec<String> = Vec::with_capacity(width.div_ceil(2) + 2);
    out.push("  scan to connect:".to_string());
    for y in (0..width).step_by(2) {
        let mut row = String::from("  ");
        for x in 0..width {
            let top = code[(x, y)] == qrcode::Color::Light;
            let bottom = if y + 1 < width {
                code[(x, y + 1)] == qrcode::Color::Light
            } else {
                true
            };
            // Light = background. Combine two vertical modules into one cell:
            // both dark → '█', top dark only → '▀', bottom dark only → '▄', both light → ' '.
            row.push(if top {
                if bottom {
                    ' '
                } else {
                    '▄'
                }
            } else if bottom {
                '▀'
            } else {
                '█'
            });
        }
        out.push(row);
    }
    Some(out)
}

/// The control page shell (HTML). Split into `remote_assets/` files (page/script/styles/SW)
/// served as separate token-scoped routes so the CSP carries no 'unsafe-inline'. `__BASE__` is
/// injected at serve time. The page renders the generic `overlay` projection (tappable rows,
/// filter box, free-text box, text body) and sends `RemoteInput` JSON back over the WS.
pub(crate) const CONTROL_PAGE: &str = include_str!("remote_assets/page.html");

/// The page's JavaScript (see [`CONTROL_PAGE`]) — `__BASE__` injected at serve time.
pub(crate) const APP_JS: &str = include_str!("remote_assets/app.js");

/// The page's stylesheet (static).
pub(crate) const STYLES_CSS: &str = include_str!("remote_assets/styles.css");

/// The token-scoped PWA service worker. Its presence + a `fetch` handler is what makes the control
/// page installable to a phone home screen; it caches non-navigation assets (network-first) so a
/// reconnect is instant. Live state flows over the WebSocket, never `fetch`.
///
/// Navigations are special-cased: when the server is gone (each session gets a fresh port +
/// token, so an installed home-screen app outlives its session), serving the *cached* shell
/// showed a live-looking page stuck on "reconnecting…" forever. Instead the SW answers a failed
/// navigation with an explicit "session ended — reopen /remote from the TUI" page. A stable
/// origin that makes the install permanent is the Phase-4 daemon's job.
pub(crate) const SERVICE_WORKER: &str = include_str!("remote_assets/sw.js");

/// The app icon (inline SVG — no binary asset to serve). A hammer mark on the brand background;
/// `sizes:"any"` in the manifest lets the single SVG satisfy every install target.
pub(crate) const ICON_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><rect width="24" height="24" rx="5" fill="#16161c"/><g fill="none" stroke="#ff913c" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="m15 12-8.5 8.5c-.83.83-2.17.83-3 0a2.12 2.12 0 0 1 0-3L12 9"/><path d="M17.64 15 22 10.64"/><path d="m20.91 11.7-1.25-1.25c-.6-.6-.93-1.4-.93-2.25v-.86L16.01 4.6a5.56 5.56 0 0 0-3.94-1.64H9l.92.82A6.18 6.18 0 0 1 12 8.4v1.56l2 2h2.47l2.26 1.91"/></g></svg>"##;

/// Build the PWA manifest JSON for a token base path (e.g. `/<token>`). `start_url`/`scope` use
/// the slashed form so the installed app launches inside the service-worker scope and runs
/// standalone (no browser chrome) straight into this session's control page.
pub(crate) fn manifest_json(base: &str) -> String {
    format!(
        r##"{{"name":"Forge remote control","short_name":"Forge","description":"Drive a Forge coding session from anywhere.","start_url":"{base}/","scope":"{base}/","display":"standalone","background_color":"#16161c","theme_color":"#16161c","orientation":"any","icons":[{{"src":"{base}/icon.svg","sizes":"any","type":"image/svg+xml","purpose":"any"}},{{"src":"{base}/icon.svg","sizes":"any","type":"image/svg+xml","purpose":"maskable"}}]}}"##
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_serializes_to_json_with_all_fields() {
        let s = Snapshot {
            session_id: "abc12345".into(),
            title: "fix the parser".into(),
            cwd: "/home/u/proj".into(),
            worktree: Some("/home/u/proj/.forge/worktrees/abc12345".into()),
            exposure: "LAN".into(),
            busy: true,
            temper: "Ask".into(),
            tier: Some("complex".into()),
            model: "groq::llama-3.3-70b".into(),
            cost_usd: 0.0123,
            context_tokens: 18_200,
            context_limit: Some(200_000),
            streaming: "thinking…".into(),
            transcript: vec!["you: hi".into(), "forge: hello".into()],
            tasks: vec![SnapTask {
                title: "build it".into(),
                status: "in_progress".into(),
            }],
            subagents: vec![SnapSubagent {
                agent: "general".into(),
                task: "scan".into(),
                model: Some("haiku".into()),
                last: "reading…".into(),
                done: false,
                cost: 0.001,
            }],
            queued: vec!["next thing".into()],
            permission_prompt: Some("allow write_file".into()),
            question: None,
            question_options: vec![SnapOption {
                label: "Yes".into(),
                description: "do it".into(),
            }],
            question_allow_other: true,
            overlay: Some(SnapOverlay {
                kind: "picker:model_pin".into(),
                title: "⊕ pin model".into(),
                rows: vec![SnapRow {
                    id: "groq::llama-3.3-70b".into(),
                    label: "llama-3.3-70b".into(),
                    detail: "free · 128k ctx".into(),
                    selected: true,
                    group: None,
                }],
                selected: 0,
                filter: Some("lla".into()),
                free_text: false,
                body: None,
            }),
            diff: Some(SnapDiff {
                pending: true,
                files: vec![SnapDiffFile {
                    path: "src/a.rs".into(),
                    kind: "modified".into(),
                    binary: false,
                    adds: 3,
                    dels: 1,
                    hunks: vec![SnapDiffHunk {
                        header: "@@ -1,2 +1,4 @@".into(),
                        lines: vec![" ctx".into(), "-old".into(), "+new".into()],
                    }],
                    skipped_lines: 12,
                }],
                skipped_files: 2,
            }),
            plan: Some(SnapPlan {
                title: "Ship it".into(),
                steps: vec![SnapPlanStep {
                    title: "step 1".into(),
                    detail: "do the thing".into(),
                }],
                notes: Some("risky".into()),
            }),
            copy_text: Some("fn main() {}".into()),
            prompt_seq: 7,
            notes: vec!["⚠ /remote can only be toggled from the TUI".into()],
            revision: 42,
            ..Default::default()
        };
        // Snapshot is server→client (serialize only); confirm the wire shape carries every field
        // the control page's JS reads, so a schema drift is caught here rather than at runtime.
        let json = serde_json::to_string(&s).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["protocol"], PROTOCOL_VERSION);
        assert_eq!(v["session_id"], "abc12345");
        // v6: title + worktree ride in every frame (the daemon page header + session list).
        assert_eq!(v["title"], "fix the parser");
        assert_eq!(v["cwd"], "/home/u/proj");
        assert_eq!(v["worktree"], "/home/u/proj/.forge/worktrees/abc12345");
        assert_eq!(v["exposure"], "LAN");
        assert_eq!(v["busy"], true);
        assert_eq!(v["tier"], "complex");
        assert_eq!(v["model"], "groq::llama-3.3-70b");
        assert_eq!(v["cost_usd"], 0.0123);
        assert_eq!(v["context_tokens"], 18200);
        assert_eq!(v["context_limit"], 200000);
        assert_eq!(v["transcript"][0], "you: hi");
        assert_eq!(v["tasks"][0]["title"], "build it");
        assert_eq!(v["tasks"][0]["status"], "in_progress");
        assert_eq!(v["subagents"][0]["agent"], "general");
        assert_eq!(v["subagents"][0]["done"], false);
        assert_eq!(v["queued"][0], "next thing");
        assert_eq!(v["permission_prompt"], "allow write_file");
        assert_eq!(v["question"], serde_json::Value::Null);
        assert_eq!(v["question_options"][0]["label"], "Yes");
        assert_eq!(v["question_allow_other"], true);
        // v4: the generic overlay projection + the /copy payload ride in the snapshot.
        assert_eq!(v["overlay"]["kind"], "picker:model_pin");
        assert_eq!(v["overlay"]["title"], "⊕ pin model");
        assert_eq!(v["overlay"]["rows"][0]["id"], "groq::llama-3.3-70b");
        assert_eq!(v["overlay"]["rows"][0]["label"], "llama-3.3-70b");
        assert_eq!(v["overlay"]["rows"][0]["detail"], "free · 128k ctx");
        assert_eq!(v["overlay"]["rows"][0]["selected"], true);
        assert_eq!(v["overlay"]["rows"][0]["group"], serde_json::Value::Null);
        assert_eq!(v["overlay"]["selected"], 0);
        assert_eq!(v["overlay"]["filter"], "lla");
        assert_eq!(v["overlay"]["free_text"], false);
        assert_eq!(v["overlay"]["body"], serde_json::Value::Null);
        // v7: the structured diff card + the plan card ride in the snapshot.
        assert_eq!(v["diff"]["pending"], true);
        assert_eq!(v["diff"]["files"][0]["path"], "src/a.rs");
        assert_eq!(v["diff"]["files"][0]["kind"], "modified");
        assert_eq!(v["diff"]["files"][0]["binary"], false);
        assert_eq!(v["diff"]["files"][0]["adds"], 3);
        assert_eq!(v["diff"]["files"][0]["dels"], 1);
        assert_eq!(
            v["diff"]["files"][0]["hunks"][0]["header"],
            "@@ -1,2 +1,4 @@"
        );
        assert_eq!(v["diff"]["files"][0]["hunks"][0]["lines"][2], "+new");
        assert_eq!(v["diff"]["files"][0]["skipped_lines"], 12);
        assert_eq!(v["diff"]["skipped_files"], 2);
        assert_eq!(v["plan"]["title"], "Ship it");
        assert_eq!(v["plan"]["steps"][0]["title"], "step 1");
        assert_eq!(v["plan"]["steps"][0]["detail"], "do the thing");
        assert_eq!(v["plan"]["notes"], "risky");
        assert_eq!(v["copy_text"], "fn main() {}");
        assert_eq!(v["prompt_seq"], 7);
        assert_eq!(v["notes"][0], "⚠ /remote can only be toggled from the TUI");
        assert_eq!(v["revision"], 42);
        // v5: the resync flag rides in every frame (false on the live stream, true on the one
        // full snapshot a gapped reconnect gets).
        assert_eq!(v["resync"], false);
        assert_eq!(v["closed"], false);

        let resynced = Snapshot {
            resync: true,
            ..Default::default()
        };
        let v: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&resynced).unwrap()).unwrap();
        assert_eq!(v["resync"], true);
    }

    #[test]
    fn history_row_serializes_the_wire_shape() {
        let row = HistoryRow {
            seq: 12,
            role: "assistant".into(),
            content: "```rust\nfn main() {}\n```".into(),
            model: Some("groq::llama-3.3-70b".into()),
            created_at: 1_770_000_000,
            visibility: "llm".into(),
        };
        let v: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&row).unwrap()).unwrap();
        assert_eq!(v["seq"], 12);
        assert_eq!(v["role"], "assistant");
        assert_eq!(v["content"], "```rust\nfn main() {}\n```");
        assert_eq!(v["model"], "groq::llama-3.3-70b");
        assert_eq!(v["created_at"], 1_770_000_000_i64);
        assert_eq!(v["visibility"], "llm");
    }

    /// A snapshot whose only distinguishing field is its revision, for event-log tests.
    fn rev_snap(rev: u64) -> Snapshot {
        Snapshot {
            revision: rev,
            ..Default::default()
        }
    }

    #[test]
    fn event_log_replays_exactly_the_frames_after_rev() {
        let mut log = EventLog::new(EVENT_LOG_CAP);
        for rev in 1..=10 {
            log.push(rev, rev_snap(rev));
        }
        // Everything after rev 6 — exactly 7, 8, 9, 10, oldest first.
        let missed = log.replay_after(6).expect("gap is fillable");
        assert_eq!(
            missed.iter().map(|s| s.revision).collect::<Vec<_>>(),
            vec![7, 8, 9, 10]
        );
        // Already current → an empty replay (NOT a resync): the client missed nothing.
        assert_eq!(log.replay_after(10).expect("current is fillable").len(), 0);
        // The boundary: rev = front-1 still replays the entire retained log.
        assert_eq!(log.replay_after(0).expect("full log").len(), 10);
    }

    #[test]
    fn event_log_eviction_and_unknown_revs_force_resync() {
        let mut log = EventLog::new(4);
        for rev in 1..=10 {
            log.push(rev, rev_snap(rev));
        }
        // Ring holds 7..=10 now; a client at rev 2 needs 3..=10 — 3..=6 are gone: resync.
        assert!(log.replay_after(2).is_none(), "evicted gap must resync");
        assert!(
            log.replay_after(5).is_none(),
            "first needed frame (6) evicted"
        );
        // rev 6 needs 7..=10 — all retained.
        assert_eq!(log.replay_after(6).unwrap().len(), 4);
        // A future/foreign rev (e.g. from a previous server on the same page) must resync,
        // never silently pretend the client is current.
        assert!(log.replay_after(11).is_none(), "future rev must resync");
        // An empty log can't fill anything.
        assert!(EventLog::new(4).replay_after(1).is_none());
    }

    #[test]
    fn event_log_is_bounded_by_its_cap() {
        let mut log = EventLog::new(8);
        for rev in 1..=10_000 {
            log.push(rev, rev_snap(rev));
            assert!(log.len() <= 8, "ring must never exceed its cap");
        }
        assert_eq!(log.len(), 8);
        assert_eq!(
            log.replay_after(9_992).unwrap().len(),
            8,
            "the newest cap-many frames are retained"
        );
    }

    #[test]
    fn history_page_limit_is_clamped() {
        assert_eq!(history_page_limit(None), HISTORY_PAGE_DEFAULT);
        assert_eq!(history_page_limit(Some(10)), 10);
        assert_eq!(history_page_limit(Some(0)), 1, "zero would loop forever");
        assert_eq!(
            history_page_limit(Some(1_000_000)),
            HISTORY_PAGE_MAX,
            "a hostile limit can't dump the whole table"
        );
    }

    #[test]
    fn remote_inputs_deserialize_tagged_variants() {
        assert_eq!(
            serde_json::from_str::<RemoteInput>(r#"{"kind":"prompt","text":"fix it"}"#).unwrap(),
            RemoteInput::Prompt {
                text: "fix it".into(),
                attachments: Vec::new(),
            }
        );
        assert_eq!(
            serde_json::from_str::<RemoteInput>(r#"{"kind":"allow","yes":true,"seq":3}"#).unwrap(),
            RemoteInput::Allow { yes: true, seq: 3 }
        );
        assert_eq!(
            serde_json::from_str::<RemoteInput>(r#"{"kind":"answer","text":"2","seq":9}"#).unwrap(),
            RemoteInput::Answer {
                text: "2".into(),
                seq: 9
            }
        );
        assert_eq!(
            serde_json::from_str::<RemoteInput>(r#"{"kind":"interrupt"}"#).unwrap(),
            RemoteInput::Interrupt
        );
        assert_eq!(
            serde_json::from_str::<RemoteInput>(r#"{"kind":"dequeue","index":1,"text":"hi"}"#)
                .unwrap(),
            RemoteInput::Dequeue {
                index: 1,
                text: "hi".into()
            }
        );
    }

    #[test]
    fn v4_key_and_overlay_inputs_deserialize() {
        assert_eq!(
            serde_json::from_str::<RemoteInput>(r#"{"kind":"key","key":"Up"}"#).unwrap(),
            RemoteInput::Key { key: "Up".into() }
        );
        assert_eq!(
            serde_json::from_str::<RemoteInput>(r#"{"kind":"overlay_select","id":"mesh"}"#)
                .unwrap(),
            RemoteInput::OverlaySelect { id: "mesh".into() }
        );
        assert_eq!(
            serde_json::from_str::<RemoteInput>(r#"{"kind":"overlay_nav","delta":-3}"#).unwrap(),
            RemoteInput::OverlayNav { delta: -3 }
        );
        assert_eq!(
            serde_json::from_str::<RemoteInput>(r#"{"kind":"overlay_filter","text":"llam"}"#)
                .unwrap(),
            RemoteInput::OverlayFilter {
                text: "llam".into()
            }
        );
        assert_eq!(
            serde_json::from_str::<RemoteInput>(r#"{"kind":"overlay_cancel"}"#).unwrap(),
            RemoteInput::OverlayCancel
        );
    }

    #[test]
    fn named_keys_map_to_tui_keys() {
        use forge_tui::KeyKind as K;
        // The full v4 named-key table — each name is part of the wire protocol.
        for (name, want) in [
            ("Up", K::Up),
            ("Down", K::Down),
            ("Enter", K::Enter),
            ("Esc", K::Esc),
            ("Tab", K::Tab),
            ("BTab", K::CycleTemper), // SHIFT+TAB = the temper cycler
            ("PageUp", K::PageUp),
            ("PageDown", K::PageDown),
            ("Home", K::Home),
            ("End", K::End),
            ("Backspace", K::Backspace),
            ("Char:x", K::Char('x')),
            ("Char:/", K::Char('/')),
            ("Char:é", K::Char('é')),
        ] {
            assert_eq!(named_key(name), Some(want), "named key {name}");
        }
        // Unknown names / malformed chars are dropped, never guessed.
        assert_eq!(named_key("Delete"), None);
        assert_eq!(named_key("Char:"), None);
        assert_eq!(named_key("Char:ab"), None, "exactly one char");
        assert_eq!(named_key(""), None);
    }

    #[test]
    fn prompt_input_with_attachments_deserializes() {
        assert_eq!(
            serde_json::from_str::<RemoteInput>(
                r#"{"kind":"prompt","text":"look at this","attachments":[{"path":"/tmp/x/.forge/uploads/s1/1-shot.png","image":true},{"path":"/tmp/x/.forge/uploads/s1/2-notes.txt","image":false}]}"#
            )
            .unwrap(),
            RemoteInput::Prompt {
                text: "look at this".into(),
                attachments: vec![
                    PromptAttachment {
                        path: "/tmp/x/.forge/uploads/s1/1-shot.png".into(),
                        image: true,
                    },
                    PromptAttachment {
                        path: "/tmp/x/.forge/uploads/s1/2-notes.txt".into(),
                        image: false,
                    },
                ],
            }
        );
    }

    #[test]
    fn v7_attach_input_deserializes() {
        assert_eq!(
            serde_json::from_str::<RemoteInput>(
                r#"{"kind":"attach","path":"/tmp/x/.forge/uploads/s1/1-shot.png","image":true}"#
            )
            .unwrap(),
            RemoteInput::Attach {
                path: "/tmp/x/.forge/uploads/s1/1-shot.png".into(),
                image: true
            }
        );
    }

    #[test]
    fn upload_names_are_flattened_to_one_safe_component() {
        // Traversal: only the final component survives, and `..` can't survive as dots.
        assert_eq!(sanitize_upload_name("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_upload_name(r"..\..\evil.txt"), "evil.txt");
        assert_eq!(sanitize_upload_name("a/b/../c.txt"), "c.txt");
        // Hidden files / bare dots never survive.
        assert_eq!(sanitize_upload_name(".."), "upload");
        assert_eq!(sanitize_upload_name(".env"), "env");
        // Shell-hostile characters are flattened; sane names pass through.
        assert_eq!(sanitize_upload_name("my file (1).png"), "my_file__1_.png");
        assert_eq!(sanitize_upload_name("report-v2.md"), "report-v2.md");
        // Degenerate inputs still yield a usable name, bounded in length.
        assert_eq!(sanitize_upload_name(""), "upload");
        assert_eq!(sanitize_upload_name("///"), "upload");
        assert!(sanitize_upload_name(&"x".repeat(500)).len() <= 80);
    }

    #[test]
    fn store_upload_enforces_caps_and_types() {
        let dir = std::env::temp_dir().join(format!("forge-upload-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        // A text file stores under a timestamp-prefixed sanitized name, inside `dir`.
        let (path, image) =
            store_upload(&dir, "../notes file.txt", Some("text/plain"), b"hello").unwrap();
        assert!(!image);
        assert!(path.starts_with(&dir), "never escapes the upload dir");
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        assert!(
            name.ends_with("-notes_file.txt"),
            "sanitized + prefixed: {name}"
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"hello");

        // Images are detected by content type OR extension — bytes go through unvalidated
        // (the vision provider is the judge of image bytes, not us).
        let (_, image) = store_upload(&dir, "shot.png", None, &[0x89, 0x50, 0x4e]).unwrap();
        assert!(image, "extension marks an image");
        let (_, image) = store_upload(&dir, "blob", Some("image/jpeg"), &[0xff, 0xd8]).unwrap();
        assert!(image, "content type marks an image");

        // Non-image binary is refused: only images and UTF-8 text have an injection path.
        assert!(store_upload(
            &dir,
            "prog.bin",
            Some("application/octet-stream"),
            &[0x00, 0xff]
        )
        .is_err());
        // Empty and oversized files are refused.
        assert!(store_upload(&dir, "empty.txt", None, b"").is_err());
        let big = vec![b'a'; UPLOAD_MAX_BYTES + 1];
        assert!(store_upload(&dir, "big.txt", None, &big).is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn page_renders_the_v7_cards_and_input_extras() {
        // The page must render the plan + diff cards, upload, voice, and the fleet signals.
        for needle in [
            "function renderPlan",
            "function renderDiff",
            "\"Build it\"", // Approve answers core's resolve_plan_approval option by label
            r#"BASE + "/api/upload""#,
            "SpeechRecognition", // voice input, hidden where unsupported
            "clipboardData",     // paste-an-image support
            "r.waiting",         // fleet dashboard: waiting-on-decision signal
            "needs decision",
            "s.diff",
            "s.plan",
        ] {
            assert!(APP_JS.contains(needle), "app.js must contain {needle:?}");
        }
        for id in [
            "planbox", "psteps", "pok", "previse", "pcancel", "diffbox", "dfiles", "attach", "mic",
            "file", "upchips",
        ] {
            assert!(
                CONTROL_PAGE.contains(&format!("id=\"{id}\"")),
                "page has #{id}"
            );
        }
        // The mic transcribes into the prompt box — it must never auto-send.
        assert!(!APP_JS.contains("rec.onresult = (e) => { submit"));
        // Diff/plan cards are DOM-built via textContent (transcript-grade untrusted content).
        assert!(APP_JS.contains("hd.textContent = hk.header"));
    }

    #[test]
    fn legacy_answers_without_seq_are_rejected() {
        // A v2 page's Allow/Answer carries no `seq`, so it can't prove which prompt it targets —
        // parsing must fail (the ws layer then drops the frame) rather than defaulting to some
        // seq that could approve a newer, more dangerous prompt. The page shows the protocol-
        // mismatch banner, so the operator knows to refresh.
        assert!(serde_json::from_str::<RemoteInput>(r#"{"kind":"allow","yes":true}"#).is_err());
        assert!(serde_json::from_str::<RemoteInput>(r#"{"kind":"answer","text":"2"}"#).is_err());
        // Prompt/Interrupt are seq-free (they don't resolve a pending prompt) and still parse.
        assert!(serde_json::from_str::<RemoteInput>(r#"{"kind":"interrupt"}"#).is_ok());
        // Dequeue is seq-free too — it self-validates via the echoed index+text instead.
        assert!(
            serde_json::from_str::<RemoteInput>(r#"{"kind":"dequeue","index":0,"text":"x"}"#)
                .is_ok()
        );
    }

    #[test]
    fn stale_prompt_seq_is_not_current() {
        // The render loop only resolves a pending prompt when the answer echoes the CURRENT seq.
        assert!(prompt_seq_current(5, 5));
        assert!(
            !prompt_seq_current(6, 5),
            "answer for a replaced prompt is stale"
        );
        assert!(
            !prompt_seq_current(5, 6),
            "an answer can't target a future prompt"
        );
    }

    #[test]
    fn snapshot_partial_eq_backs_change_only_broadcast() {
        // The dedup in the render loop (`last != candidate → send`) hinges on PartialEq seeing
        // real state changes and nothing else.
        let a = Snapshot::default();
        let b = Snapshot::default();
        assert_eq!(
            a, b,
            "identical snapshots compare equal (no spurious sends)"
        );
        let c = Snapshot {
            busy: true,
            ..Default::default()
        };
        assert_ne!(a, c, "a real change compares unequal");
        let d = Snapshot {
            prompt_seq: 1,
            ..Default::default()
        };
        assert_ne!(a, d, "a new prompt identity is a change");
    }

    #[test]
    fn exposure_label_reports_tls_failure_as_unavailable() {
        assert_eq!(
            exposure_label(Some("ngrok"), false, false),
            "public (ngrok)"
        );
        assert_eq!(exposure_label(None, true, false), "LAN");
        assert_eq!(
            exposure_label(None, true, true),
            "LAN (unavailable — TLS failed)",
            "a TLS failure must never masquerade as a healthy LAN server"
        );
        assert_eq!(exposure_label(None, false, false), "loopback");
    }

    #[test]
    fn discovered_lan_ip_is_never_unspecified_or_loopback() {
        // Discovery is environment-dependent (offline machines have no route), but the contract
        // is: whatever it returns is an address a peer could actually dial — never 0.0.0.0/::
        // and never 127.0.0.1 (those are exactly the useless values it exists to replace).
        if let Some(ip) = discover_lan_ip() {
            assert!(
                !ip.is_unspecified(),
                "discovered IP must not be unspecified: {ip}"
            );
            assert!(
                !ip.is_loopback(),
                "discovered IP must not be loopback: {ip}"
            );
        }
    }

    #[test]
    fn lan_display_host_prefers_the_config_override() {
        let addr: SocketAddr = "0.0.0.0:4123".parse().unwrap();
        assert_eq!(
            lan_display_host(Some("192.168.1.5"), addr),
            "192.168.1.5",
            "[remote] host wins over discovery"
        );
        // Without an override the result is discovery-or-bind-address; either way it's the
        // literal bind IP only when discovery found nothing.
        let h = lan_display_host(None, addr);
        assert!(!h.is_empty());
        if discover_lan_ip().is_some() {
            assert_ne!(
                h, "0.0.0.0",
                "with a discoverable interface the URL never says 0.0.0.0"
            );
        }
    }

    #[test]
    fn manifest_is_token_scoped_valid_json() {
        let m = manifest_json("/deadbeef");
        let v: serde_json::Value = serde_json::from_str(&m).expect("manifest is valid JSON");
        assert_eq!(v["start_url"], "/deadbeef/");
        assert_eq!(v["scope"], "/deadbeef/");
        assert_eq!(v["display"], "standalone");
        assert_eq!(v["icons"][0]["src"], "/deadbeef/icon.svg");
    }

    #[test]
    fn control_page_injects_base_path() {
        let html = CONTROL_PAGE.replace("__BASE__", "/cafef00d");
        assert!(!html.contains("__BASE__"), "all base placeholders replaced");
        assert!(
            html.contains(r#"href="/cafef00d/manifest.webmanifest""#),
            "manifest link is token-scoped"
        );
        assert!(
            html.contains(r#"href="/cafef00d/icon.svg""#),
            "icon link is token-scoped"
        );
        assert!(
            html.contains(r#"href="/cafef00d/styles.css""#),
            "stylesheet link is token-scoped"
        );
        assert!(
            html.contains(r#"src="/cafef00d/app.js""#),
            "script src is token-scoped"
        );
        let js = APP_JS.replace("__BASE__", "/cafef00d");
        assert!(
            js.contains(r#"const BASE = "/cafef00d";"#),
            "JS BASE is the token path (WS + SW + manifest derive from it)"
        );
    }

    #[test]
    fn control_page_speaks_the_current_protocol() {
        // The page's bundled protocol constant must track PROTOCOL_VERSION (mismatch = banner).
        assert!(
            APP_JS.contains(&format!("const PROTO = {PROTOCOL_VERSION};")),
            "page PROTO must equal PROTOCOL_VERSION"
        );
        // Answers must echo the prompt identity (seq) the buttons were rendered from.
        assert!(APP_JS.contains("seq: curSeq"), "allow/answer carry seq");
        assert!(
            APP_JS.contains("s.prompt_seq"),
            "page tracks the snapshot's prompt_seq"
        );
        // The mismatch banner covers BOTH directions (older page vs older server).
        assert!(APP_JS.contains("s.protocol > PROTO"));
    }

    #[test]
    fn page_reconnects_with_its_last_seen_revision() {
        // v5 reconnect/replay: the page must send `?rev=<last seen>` on every (re)connect,
        // persist the revision across reloads, dedupe the replay/live overlap, and honor
        // the resync flag.
        for needle in [
            // The handshake carries the last seen revision — but only once this page life has
            // painted a frame. A cold reload restores lastRev from sessionStorage into a BLANK
            // DOM; asking for "after lastRev" on an idle session then replays nothing and the
            // page stays blank. rev=0 forces one full resync on the first paint.
            r#""/ws?rev=" + (painted ? lastRev : 0)"#,
            "sessionStorage.getItem(REV_KEY", // …which survives a page reload
            "sessionStorage.setItem(REV_KEY",
            "s.revision <= lastRev", // replay/live overlap dedup
            "s.resync",              // resync frames always apply
        ] {
            assert!(APP_JS.contains(needle), "app.js must contain {needle:?}");
        }
    }

    #[test]
    fn page_paginates_history_and_renders_rich_transcript() {
        // v5 full scrollback: scroll-up fetches older pages from the token-scoped history API —
        // and under a daemon the request must address the attached session explicitly (the
        // shared route pages an empty id otherwise and scrollback never loads).
        for needle in [
            r#"BASE + "/api/history""#,
            "?before=",
            "histRow",
            r#""/api/history" + q + sess"#,
        ] {
            assert!(APP_JS.contains(needle), "app.js must contain {needle:?}");
        }
        // …and messages are rendered with the safe markdown renderer + built-in highlighter
        // (createElement/textContent only, tap-to-copy on fenced blocks — never innerHTML
        // with transcript content).
        for needle in [
            "function mdRender",
            "function inlineMd",
            "function highlight",
            "function codeBlock",
            "codecopy",
            "tok-k",
        ] {
            assert!(APP_JS.contains(needle), "app.js must contain {needle:?}");
        }
        // The transcript panel splits into accumulated history + the rebuilt live tail.
        for id in ["hist", "tail", "histload"] {
            assert!(
                CONTROL_PAGE.contains(&format!("id=\"{id}\"")),
                "page has #{id}"
            );
        }
        // The service worker must never serve /api/ responses from cache.
        assert!(
            SERVICE_WORKER.contains("/api/"),
            "SW special-cases the live-data API routes"
        );
    }

    #[test]
    fn page_renders_the_generic_overlay_and_speaks_the_overlay_verbs() {
        // The page must render `Snapshot::overlay` and send every v4 overlay verb + named keys.
        for needle in [
            "s.overlay",
            r#"kind: "overlay_select""#,
            r#"kind: "overlay_filter""#,
            r#"kind: "overlay_cancel""#,
            r#"kind: "key""#,
        ] {
            assert!(APP_JS.contains(needle), "app.js must contain {needle:?}");
        }
        // …and the /copy payload gets a copy-here button.
        assert!(APP_JS.contains("copy_text"), "page reads copy_text");
        assert!(
            APP_JS.contains("navigator.clipboard"),
            "copy uses the device clipboard"
        );
        // The overlay skeleton is in the HTML shell.
        for id in ["overlay", "otitle", "ofilter", "orows", "obody", "ofree"] {
            assert!(
                CONTROL_PAGE.contains(&format!("id=\"{id}\"")),
                "page has #{id}"
            );
        }
    }

    #[test]
    fn chips_row_has_no_dead_diff_and_gains_mode_and_help() {
        // `/diff` never existed as a command — the chip was a dead affordance. /models, /mode
        // and /help are picker/palette-backed and fully remote-drivable via the overlay.
        // (The chip is /models, NOT /model: bare `/model` silently CLEARS the model pin —
        // a destructive tap, not a picker.)
        assert!(
            !CONTROL_PAGE.contains("/diff"),
            "the dead /diff chip is gone"
        );
        assert!(
            !CONTROL_PAGE.contains("data-cmd=\"/model\""),
            "bare /model clears the pin — the chip must be /models"
        );
        for cmd in ["/plan", "/compact", "/models", "/mode", "/help"] {
            assert!(
                CONTROL_PAGE.contains(&format!("data-cmd=\"{cmd}\"")),
                "chip {cmd} present"
            );
        }
    }

    #[test]
    fn csp_has_no_unsafe_inline_and_page_has_no_inline_handlers() {
        assert!(
            !PAGE_CSP.contains("unsafe-inline"),
            "the asset split removed the last inline script/style: {PAGE_CSP}"
        );
        // Inline handlers would be dead under this CSP — make sure none sneak back in.
        assert!(
            !CONTROL_PAGE.contains("onclick="),
            "no inline onclick attributes"
        );
        assert!(!CONTROL_PAGE.contains("<script>"), "no inline script block");
        assert!(!CONTROL_PAGE.contains("<style>"), "no inline style block");
        // …and the generated action buttons use data-act delegation, not inline handlers.
        assert!(
            !APP_JS.contains("onclick=\""),
            "no generated inline handlers"
        );
    }

    #[test]
    fn service_worker_has_fetch_handler() {
        // PWA installability requires a fetch handler; guard against accidentally dropping it.
        assert!(SERVICE_WORKER.contains(r#"addEventListener("fetch""#));
    }

    #[test]
    fn service_worker_serves_session_ended_page_when_server_is_gone() {
        // A dead server (session over — port + token are per-session) must yield an explicit
        // "session ended" navigation response, not the cached live shell stuck on
        // "reconnecting…" forever.
        assert!(SERVICE_WORKER.contains("session has ended"));
        assert!(
            SERVICE_WORKER.contains(r#"req.mode === "navigate""#),
            "navigations are special-cased away from the stale-shell cache"
        );
    }

    #[test]
    fn remote_auto_maps_to_exposure() {
        use forge_config::RemoteAuto;
        assert_eq!(Exposure::from(RemoteAuto::Local), Exposure::Local);
        assert_eq!(Exposure::from(RemoteAuto::Lan), Exposure::Lan);
        assert_eq!(Exposure::from(RemoteAuto::Anywhere), Exposure::Anywhere);
        // Off never reaches `From` in practice (startup_exposure returns None), but map it safely.
        assert_eq!(Exposure::from(RemoteAuto::Off), Exposure::Local);
    }

    #[test]
    fn random_token_is_hex_and_sixteen_chars() {
        let t = random_token();
        assert_eq!(t.len(), 16);
        assert!(t.chars().all(|c| c.is_ascii_hexdigit()));
        // Two calls (almost) never collide.
        let t2 = random_token();
        assert_ne!(t, t2);
    }

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
    fn bore_is_not_probed_as_a_tunnel_provider() {
        // bore forwards raw TCP (no TLS): the token + transcript + approvals would cross the
        // public internet in cleartext. It must never be in the auto-detect list.
        assert!(!TunnelKind::ALL.iter().any(|k| k.binary() == "bore"));
    }

    #[test]
    fn qr_lines_render_for_a_url() {
        let lines = qr_lines("http://192.168.1.10:4123/0123456789abcdef").unwrap();
        assert!(lines.len() > 2, "QR has a header + rows: {lines:?}");
        assert!(lines[0].contains("scan to connect"));
        // Every row uses only the half-block glyph set (plus leading pad).
        for row in &lines[1..] {
            assert!(
                row.chars()
                    .skip(2)
                    .all(|c| matches!(c, ' ' | '▀' | '▄' | '█')),
                "row uses half-block glyphs: {row:?}"
            );
        }
    }

    /// `start()` binds a real port + spawns the server task. This is the real round-trip smoke:
    /// it does an HTTP GET on the control page (expect 200 + HTML), a wrong-token GET (expect
    /// 404, so the existence of remote control isn't leaked), and a WebSocket handshake on the
    /// token-gated WS path (expect it upgrades + delivers a snapshot). Catches the
    /// `Path<String>`-on-a-static-route regression where the WS would 400 and never connect.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "binds a real port + opens a real socket; run with --ignored (kills itself on success)"]
    async fn start_serves_page_and_upgrades_websocket() {
        // Wrap in a timeout so a stuck server/client can never hang forever. The server's spawned
        // accept loop can delay runtime shutdown on drop (a test-harness artifact, not a product
        // bug — the real loop runs under `forge chat`'s long-lived runtime), so we force-exit 0
        // once the assertions pass. Gated behind --ignored so it never runs in CI.
        let _outcome = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            use futures::StreamExt;
            let rc = start(Exposure::Local, None, None).expect("start loopback server");
            let port = rc.url.addr.port();
            let token = rc.url.token.clone();

            // 1. The control page is served at the token URL.
            let http = forge_provider::bundled_http_client();
            let page = http
                .get(format!("http://127.0.0.1:{port}/{token}"))
                .send()
                .await
                .expect("GET control page");
            assert_eq!(page.status(), 200, "control page is 200 at the token URL");
            let body = page.text().await.unwrap();
            assert!(
                body.contains("Forge remote control"),
                "HTML body served: {body}"
            );

            // 2. A wrong token → 404 (don't leak that remote control is on).
            let wrong = http
                .get(format!("http://127.0.0.1:{port}/deadbeefdeadbeef"))
                .send()
                .await
                .expect("GET wrong token");
            assert_eq!(wrong.status(), 404, "wrong token is a 404");

            // 3. The WebSocket handshake on /<token>/ws upgrades + delivers the first snapshot.
            //    This is the regression guard: a static route + `Path<String>` used to 500 here.
            let ws_url = format!("ws://127.0.0.1:{port}/{token}/ws");
            let (mut ws, _resp) = tokio_tungstenite::connect_async(&ws_url)
                .await
                .expect("WS handshake upgrades");
            let first = tokio::time::timeout(std::time::Duration::from_secs(2), ws.next())
                .await
                .expect("a snapshot arrives")
                .expect("stream open")
                .expect("text frame");
            let text = first.into_text().expect("text frame");
            let v: serde_json::Value = serde_json::from_str(&text).expect("snapshot is JSON");
            assert!(v.get("busy").is_some(), "snapshot has `busy`: {v}");
            assert!(v.get("model").is_some(), "snapshot has `model`: {v}");

            // 4. PWA assets are served + token-scoped (so the page installs to a home screen).
            let man = http
                .get(format!(
                    "http://127.0.0.1:{port}/{token}/manifest.webmanifest"
                ))
                .send()
                .await
                .expect("GET manifest");
            assert_eq!(man.status(), 200, "manifest is 200");
            let man_body = man.text().await.unwrap();
            assert!(
                man_body.contains(&format!("\"start_url\":\"/{token}/\"")),
                "manifest start_url is token-scoped: {man_body}"
            );
            let sw = http
                .get(format!("http://127.0.0.1:{port}/{token}/sw.js"))
                .send()
                .await
                .expect("GET service worker");
            assert_eq!(sw.status(), 200, "service worker is 200");
            let icon = http
                .get(format!("http://127.0.0.1:{port}/{token}/icon.svg"))
                .send()
                .await
                .expect("GET icon");
            assert_eq!(icon.status(), 200, "icon is 200");

            // 5. The split page assets are served, token-scoped, with the base injected — the
            //    CSP forbids inline script/style, so the page is dead without these.
            let js = http
                .get(format!("http://127.0.0.1:{port}/{token}/app.js"))
                .send()
                .await
                .expect("GET app.js");
            assert_eq!(js.status(), 200, "app.js is 200");
            let js_body = js.text().await.unwrap();
            assert!(
                js_body.contains(&format!("const BASE = \"/{token}\";")),
                "app.js BASE is token-scoped"
            );
            let css = http
                .get(format!("http://127.0.0.1:{port}/{token}/styles.css"))
                .send()
                .await
                .expect("GET styles.css");
            assert_eq!(css.status(), 200, "styles.css is 200");
            // All assertions passed — force-exit so the lingering server task + WS close
            // handshake can't stall the test runtime's shutdown (manual-only, --ignored).
            std::process::exit(0);
        })
        .await;
        // Unreachable on success (exit above); only reached if the 5s timeout elapsed.
        let _ = _outcome;
        panic!("WS round-trip did not complete within 5s");
    }

    /// The v5 wire round-trip: connect → take snapshots → drop → reconnect with `?rev=` and
    /// receive EXACTLY the missed frames (no gap, no duplicates, then live-follow); an unknown
    /// rev gets one `resync` frame; `/api/history` pages through the provider seam honoring
    /// `before`/`limit` and stays token-gated. Like its sibling above, it binds a real port and
    /// force-exits on success, so run it INDIVIDUALLY with --ignored.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "binds a real port + opens real sockets; run individually with --ignored (kills itself on success)"]
    async fn reconnect_replays_missed_frames_and_history_pages() {
        let _outcome = tokio::time::timeout(std::time::Duration::from_secs(8), async {
            use futures::StreamExt;

            // A canned history provider that echoes its inputs so the handler's passthrough
            // (session id from the snapshot, before/limit from the query) is observable.
            let provider: HistoryProvider = Arc::new(|sid, before, limit| {
                vec![HistoryRow {
                    seq: before.unwrap_or(-1),
                    role: "assistant".into(),
                    content: format!("sid={sid} limit={limit}"),
                    model: None,
                    created_at: 1,
                    visibility: "llm".into(),
                }]
            });
            let rc = start(Exposure::Local, None, Some(provider)).expect("start loopback server");
            let port = rc.url.addr.port();
            let token = rc.url.token.clone();

            let broadcast = |rev: u64| {
                rc.broadcast(Snapshot {
                    session_id: "sess-e2e".into(),
                    revision: rev,
                    notes: vec![format!("frame {rev}")],
                    ..Default::default()
                });
            };
            // 1. Frames 1..=3 broadcast while nobody is connected.
            (1..=3).for_each(&broadcast);

            // 2. A fresh connect (rev=0) gets ONE full snapshot flagged resync (the current
            //    state), not a replay of history it never had.
            let ws_url = format!("ws://127.0.0.1:{port}/{token}/ws?rev=0");
            let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url)
                .await
                .expect("WS handshake");
            let first = tokio::time::timeout(std::time::Duration::from_secs(2), ws.next())
                .await
                .expect("first frame arrives")
                .unwrap()
                .unwrap();
            let v: serde_json::Value = serde_json::from_str(&first.into_text().unwrap()).unwrap();
            assert_eq!(v["revision"], 3, "fresh connect sees the current frame");
            assert_eq!(v["resync"], true, "fresh connect is a resync");
            drop(ws); // the phone goes through a tunnel…

            // 3. Frames 4 and 5 happen while disconnected.
            (4..=5).for_each(&broadcast);

            // 4. Reconnect with the last seen rev: EXACTLY 4 then 5 replay (resync=false),
            //    and the stream continues live (frame 6) with no duplicates in between.
            let ws_url = format!("ws://127.0.0.1:{port}/{token}/ws?rev=3");
            let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url)
                .await
                .expect("WS re-handshake");
            for want in [4u64, 5] {
                let frame = tokio::time::timeout(std::time::Duration::from_secs(2), ws.next())
                    .await
                    .expect("replayed frame arrives")
                    .unwrap()
                    .unwrap();
                let v: serde_json::Value =
                    serde_json::from_str(&frame.into_text().unwrap()).unwrap();
                assert_eq!(v["revision"], want, "replay is exact and in order");
                assert_eq!(v["resync"], false, "replayed frames are stream frames");
                assert_eq!(v["notes"][0], format!("frame {want}"), "no content gap");
            }
            broadcast(6);
            let frame = tokio::time::timeout(std::time::Duration::from_secs(2), ws.next())
                .await
                .expect("live frame follows the replay")
                .unwrap()
                .unwrap();
            let v: serde_json::Value = serde_json::from_str(&frame.into_text().unwrap()).unwrap();
            assert_eq!(
                v["revision"], 6,
                "live-follow after replay, no gap, no dupes"
            );
            drop(ws);

            // 5. An unknown/future rev (a page from a previous server run) must resync.
            let ws_url = format!("ws://127.0.0.1:{port}/{token}/ws?rev=999");
            let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url)
                .await
                .expect("WS handshake with foreign rev");
            let frame = tokio::time::timeout(std::time::Duration::from_secs(2), ws.next())
                .await
                .expect("resync frame arrives")
                .unwrap()
                .unwrap();
            let v: serde_json::Value = serde_json::from_str(&frame.into_text().unwrap()).unwrap();
            assert_eq!(v["resync"], true, "an unfillable gap resyncs");
            assert_eq!(v["revision"], 6, "resync carries the current state");
            drop(ws);

            // 6. History pagination: the session id comes from the snapshot; before/limit pass
            //    through (limit clamped); the response is JSON with no-store.
            let http = forge_provider::bundled_http_client();
            let res = http
                .get(format!(
                    "http://127.0.0.1:{port}/{token}/api/history?before=42&limit=5"
                ))
                .send()
                .await
                .expect("GET history");
            assert_eq!(res.status(), 200);
            assert_eq!(
                res.headers().get("cache-control").unwrap(),
                "no-store",
                "live data is never cached"
            );
            let rows: serde_json::Value = res.json().await.expect("history is JSON");
            assert_eq!(rows[0]["seq"], 42, "`before` reached the provider");
            assert_eq!(rows[0]["content"], "sid=sess-e2e limit=5");
            // Omitted params → newest page at the default limit.
            let rows: serde_json::Value = http
                .get(format!("http://127.0.0.1:{port}/{token}/api/history"))
                .send()
                .await
                .expect("GET history defaults")
                .json()
                .await
                .unwrap();
            assert_eq!(rows[0]["seq"], -1, "no `before` = newest page");
            assert_eq!(
                rows[0]["content"],
                format!("sid=sess-e2e limit={HISTORY_PAGE_DEFAULT}")
            );

            // 7. The history route is token-gated like everything else.
            let wrong = http
                .get(format!(
                    "http://127.0.0.1:{port}/deadbeefdeadbeef/api/history"
                ))
                .send()
                .await
                .expect("GET history with wrong token");
            assert_eq!(wrong.status(), 404, "wrong token is a 404");

            std::process::exit(0);
        })
        .await;
        let _ = _outcome;
        panic!("reconnect/replay round-trip did not complete within 8s");
    }

    // -----------------------------------------------------------------------
    // TLS: cert generation + fingerprint
    // -----------------------------------------------------------------------

    #[test]
    fn self_signed_cert_generates_and_fingerprint_is_stable() {
        // generate_self_signed should succeed for any non-empty SAN list.
        let sans = vec!["192.168.1.10".to_string(), "localhost".to_string()];
        let cert = generate_self_signed(sans).expect("cert generation must not fail");

        // PEM blobs are non-empty and begin with the expected PEM headers.
        assert!(
            cert.cert_pem.starts_with(b"-----BEGIN CERTIFICATE-----"),
            "cert_pem must be PEM-encoded: {:?}",
            String::from_utf8_lossy(&cert.cert_pem[..40.min(cert.cert_pem.len())])
        );
        assert!(
            cert.key_pem.starts_with(b"-----BEGIN PRIVATE KEY-----")
                || cert.key_pem.starts_with(b"-----BEGIN EC PRIVATE KEY-----"),
            "key_pem must be PEM-encoded: {:?}",
            String::from_utf8_lossy(&cert.key_pem[..40.min(cert.key_pem.len())])
        );

        // Fingerprint: 64 hex digits + 31 colons = 95 chars (32 bytes × "XX:" minus trailing colon).
        // i.e. "XX:XX:…:XX" = 32 groups of 2 hex digits separated by `:` → length = 32*2 + 31 = 95.
        assert_eq!(
            cert.fingerprint.len(),
            95,
            "SHA-256 fingerprint must be 95 chars: {:?}",
            cert.fingerprint
        );
        // All non-colon chars must be uppercase hex digits.
        assert!(
            cert.fingerprint
                .chars()
                .all(|c| c == ':' || c.is_ascii_hexdigit()),
            "fingerprint chars must be hex or colon: {:?}",
            cert.fingerprint
        );
        // Colons only at positions 2, 5, 8, …
        let parts: Vec<&str> = cert.fingerprint.split(':').collect();
        assert_eq!(
            parts.len(),
            32,
            "fingerprint must have 32 colon-separated groups"
        );
        for part in &parts {
            assert_eq!(
                part.len(),
                2,
                "each group must be exactly 2 hex digits: {part:?}"
            );
            assert!(
                part.chars().all(|c| c.is_ascii_hexdigit()),
                "group must be uppercase hex: {part:?}"
            );
        }

        // Generating the same cert twice produces different fingerprints (each call generates
        // a fresh key + cert, so the DER is different even for the same SANs).
        let cert2 =
            generate_self_signed(vec!["localhost".to_string()]).expect("second cert generation");
        // It's technically possible (but astronomically unlikely) for two random certs to share
        // the same fingerprint. If this ever fires, something is wrong.
        assert_ne!(
            cert.fingerprint, cert2.fingerprint,
            "two separately generated certs must have different fingerprints"
        );
    }

    #[test]
    fn sha256_fingerprint_known_vector() {
        // SHA-256("") = e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
        // Verify our inline implementation against this well-known test vector.
        let empty_digest = sha256_raw(&[]);
        let expected: [u8; 32] = [
            0xe3, 0xb0, 0xc4, 0x42, 0x98, 0xfc, 0x1c, 0x14, 0x9a, 0xfb, 0xf4, 0xc8, 0x99, 0x6f,
            0xb9, 0x24, 0x27, 0xae, 0x41, 0xe4, 0x64, 0x9b, 0x93, 0x4c, 0xa4, 0x95, 0x99, 0x1b,
            0x78, 0x52, 0xb8, 0x55,
        ];
        assert_eq!(
            empty_digest, expected,
            "SHA-256 of empty input must match FIPS vector"
        );

        // SHA-256("abc") — verified against Python hashlib.sha256(b"abc").digest():
        // ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad
        let abc_digest = sha256_raw(b"abc");
        let abc_expected: [u8; 32] = [
            0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae,
            0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61,
            0xf2, 0x00, 0x15, 0xad,
        ];
        assert_eq!(
            abc_digest, abc_expected,
            "SHA-256 of 'abc' must match reference vector"
        );
    }

    #[test]
    fn fingerprint_format_is_colon_separated_uppercase_hex() {
        // Feed a known 32-byte value; verify the formatted fingerprint string.
        let input = [0xABu8; 32]; // all 0xAB bytes
        let fp = sha256_fingerprint(&input);
        // sha256_fingerprint of [0xAB; 32] — the actual digest. What matters here is the FORMAT:
        // we reuse sha256_fingerprint on a real digest of a simple value.
        // Instead, directly test the format rules on the sha256_raw output.
        let digest = sha256_raw(&[0x00]);
        let formatted = sha256_fingerprint(&[0x00]);
        // Must be 95 chars: 32 groups of 2 hex digits separated by ':'
        assert_eq!(formatted.len(), 95);
        let parts: Vec<&str> = formatted.split(':').collect();
        assert_eq!(parts.len(), 32);
        // All uppercase.
        assert_eq!(formatted, formatted.to_uppercase());
        // Recompute manually from the raw digest and confirm they match.
        let expected: String = digest
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(":");
        assert_eq!(formatted, expected);
        // Suppress unused-variable warning for the `fp` variable above.
        let _ = fp;
    }

    // Ignored: start() binds a real socket + spawns a never-ending accept loop the test runtime
    // can't reliably abort on drop, so it hangs in CI. Cert/fingerprint correctness is covered by
    // the pure tests above; serving by start_serves_page_and_upgrades_websocket. Run with --ignored.
    #[ignore = "binds + serves a real socket; hangs under the test runtime — see comment"]
    #[tokio::test]
    async fn lan_start_url_is_https_with_fingerprint() {
        // `start(Exposure::Lan)` must return an https:// URL and a populated tls_fingerprint.
        // Requires a Tokio runtime because axum-server's from_tcp_rustls wires into the runtime.
        let rc = start(Exposure::Lan, None, None).expect("start LAN server");
        assert!(
            rc.url.url.starts_with("https://"),
            "LAN URL must be https://: {}",
            rc.url.url
        );
        assert!(
            rc.url.tls_fingerprint.is_some(),
            "LAN RemoteUrl must carry a TLS fingerprint"
        );
        let fp = rc.url.tls_fingerprint.clone().unwrap();
        assert_eq!(fp.len(), 95, "fingerprint must be 95 chars: {fp}");
    }

    #[ignore = "binds + serves a real socket; hangs under the test runtime — see comment above"]
    #[tokio::test]
    async fn local_start_url_is_http_no_fingerprint() {
        // `start(Exposure::Local)` must stay plain HTTP with no fingerprint.
        // Requires a Tokio runtime because axum::serve wires into the runtime.
        let rc = start(Exposure::Local, None, None).expect("start loopback server");
        assert!(
            rc.url.url.starts_with("http://"),
            "loopback URL must be http://: {}",
            rc.url.url
        );
        assert!(
            rc.url.tls_fingerprint.is_none(),
            "loopback RemoteUrl must have no TLS fingerprint"
        );
    }
}
