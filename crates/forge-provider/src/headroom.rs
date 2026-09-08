//! Headroom proxy routing for direct-API calls — docs/features/token-savings.md.
//!
//! [Headroom](https://github.com/chopratejas/headroom) is a local HTTP proxy that sits between an
//! LLM client and the provider and compresses what it forwards (tool results, repeated prior
//! turns, oversized file dumps) while keeping the provider's prefix cache warm. It speaks the
//! OpenAI, Anthropic and Gemini wire formats and forwards to any OpenAI-compatible upstream named
//! in an `x-headroom-base-url` request header. Forge already talks to a dozen such upstreams
//! through one genai client, so routing is a per-request retarget in the client's
//! service-target resolver rather than a per-provider setting.
//!
//! Decided once at startup ([`configure`]): `[mesh] headroom = auto` (default) turns routing on
//! only when a healthy proxy answers at `headroom_url`; `on` routes regardless (a dead proxy then
//! fails loudly instead of silently bypassing); `off` never routes.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use forge_config::{AutoToggle, MeshConfig, HEADROOM_DEFAULT_URL};

/// Decide routing for this process from `[mesh]` and record it for the provider layer.
/// Returns the proxy URL when routing is active.
pub fn configure(mesh: &MeshConfig) -> Option<String> {
    let url = mesh
        .headroom_url
        .clone()
        .unwrap_or_else(|| HEADROOM_DEFAULT_URL.to_string());
    let url = url.trim_end_matches('/').to_string();
    let active = match mesh.headroom {
        AutoToggle::Off => false,
        AutoToggle::On => true,
        AutoToggle::Auto => probe(&url, Duration::from_millis(250)),
    };
    forge_config::set_headroom_route(active.then(|| url.clone()));
    active.then_some(url)
}

/// `GET /health` against the proxy with a hard budget. Loopback answers in a millisecond; a
/// proxy that is not running refuses the connection immediately, so `auto` costs nothing when
/// Headroom is absent. Only `http://` targets are probed (the proxy is a loopback service).
pub fn probe(base_url: &str, budget: Duration) -> bool {
    let Some(rest) = base_url.strip_prefix("http://") else {
        return false;
    };
    let host_port = rest.split('/').next().unwrap_or(rest);
    let authority = if host_port.contains(':') {
        host_port.to_string()
    } else {
        format!("{host_port}:80")
    };
    let Ok(mut addrs) = authority.to_socket_addrs() else {
        return false;
    };
    let Some(addr) = addrs.next() else {
        return false;
    };
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, budget) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(budget));
    let _ = stream.set_write_timeout(Some(budget));
    let request = format!("GET /health HTTP/1.0\r\nHost: {host_port}\r\nConnection: close\r\n\r\n");
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }
    let mut buf = [0u8; 256];
    let n = stream.read(&mut buf).unwrap_or(0);
    let head = String::from_utf8_lossy(&buf[..n]);
    head.starts_with("HTTP/1.") && head.contains(" 200 ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn serve_once(status: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = listener.accept() {
                let mut buf = [0u8; 512];
                let _ = s.read(&mut buf);
                let _ = s.write_all(
                    format!("HTTP/1.0 {status}\r\nContent-Length: 2\r\n\r\nok").as_bytes(),
                );
            }
        });
        format!("http://{addr}")
    }

    #[test]
    fn probe_accepts_a_200_and_rejects_everything_else() {
        assert!(probe(&serve_once("200 OK"), Duration::from_secs(2)));
        assert!(!probe(
            &serve_once("503 Unavailable"),
            Duration::from_secs(2)
        ));
        // Nothing listening: refused immediately.
        assert!(!probe("http://127.0.0.1:1", Duration::from_millis(200)));
        assert!(!probe(
            "https://example.invalid",
            Duration::from_millis(200)
        ));
    }

    #[test]
    fn configure_honours_on_and_off_without_probing() {
        let mut mesh = forge_config::Config::default().mesh;
        mesh.headroom = AutoToggle::Off;
        mesh.headroom_url = Some("http://127.0.0.1:1/".into());
        assert_eq!(configure(&mesh), None);
        mesh.headroom = AutoToggle::On;
        assert_eq!(configure(&mesh).as_deref(), Some("http://127.0.0.1:1"));
    }
}
