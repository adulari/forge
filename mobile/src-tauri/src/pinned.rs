// LAN trust for the desktop shell. The daemon's `--lan` certificate is self-signed, so the
// WebView rejects it; instead the pairing link carries the leaf's SHA-256 and this module dials
// the daemon itself, accepting exactly that leaf. `pinned_proxy` exposes the verified connection
// as a loopback port so the existing HTTP and WebSocket code paths work unchanged.
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tokio::io::{copy_bidirectional, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::rustls::client::danger::{
    HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
};
use tokio_rustls::rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use tokio_rustls::rustls::{
    ClientConfig, DigitallySignedStruct, Error as TlsError, SignatureScheme,
};
use tokio_rustls::TlsConnector;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(6);

pub fn leaf_fingerprint(der: &[u8]) -> String {
    let digest = Sha256::digest(der);
    let mut out = String::from("sha256:");
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

pub fn normalize_fingerprint(raw: &str) -> Option<String> {
    let lowered = raw.trim().to_ascii_lowercase();
    let body = lowered
        .strip_prefix("sha256:")
        .unwrap_or(&lowered)
        .replace(':', "");
    (body.len() == 64 && body.chars().all(|c| c.is_ascii_hexdigit()))
        .then(|| format!("sha256:{body}"))
}

#[derive(Debug)]
struct PinnedVerifier {
    expected: String,
    provider: Arc<tokio_rustls::rustls::crypto::CryptoProvider>,
}

impl ServerCertVerifier for PinnedVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        if leaf_fingerprint(end_entity.as_ref()) == self.expected {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(TlsError::General("certificate fingerprint changed".into()))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        tokio_rustls::rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        tokio_rustls::rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

fn connector(expected: String) -> TlsConnector {
    let provider = Arc::new(tokio_rustls::rustls::crypto::ring::default_provider());
    let config = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .expect("ring supports the default protocol versions")
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PinnedVerifier { expected, provider }))
        .with_no_client_auth();
    TlsConnector::from(Arc::new(config))
}

#[derive(Debug, PartialEq, Eq)]
pub enum PinOutcome {
    Match,
    Mismatch,
    Unreachable(String),
}

pub async fn probe(host: &str, port: u16, expected: &str) -> PinOutcome {
    let Some(expected) = normalize_fingerprint(expected) else {
        return PinOutcome::Unreachable("malformed fingerprint".into());
    };
    match tls_connect(host, port, expected).await {
        Ok(mut stream) => {
            let _ = stream.shutdown().await;
            PinOutcome::Match
        }
        Err(Failure::Mismatch) => PinOutcome::Mismatch,
        Err(Failure::Other(message)) => PinOutcome::Unreachable(message),
    }
}

enum Failure {
    Mismatch,
    Other(String),
}

async fn tls_connect(
    host: &str,
    port: u16,
    expected: String,
) -> Result<tokio_rustls::client::TlsStream<TcpStream>, Failure> {
    let tcp = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect((host, port)))
        .await
        .map_err(|_| Failure::Other("timed out".into()))?
        .map_err(|e| Failure::Other(e.to_string()))?;
    let name = ServerName::try_from(host.to_string()).map_err(|e| Failure::Other(e.to_string()))?;
    connector(expected).connect(name, tcp).await.map_err(|e| {
        if e.to_string().contains("fingerprint changed") {
            Failure::Mismatch
        } else {
            Failure::Other(e.to_string())
        }
    })
}

fn proxies() -> &'static Mutex<HashMap<String, u16>> {
    static PROXIES: OnceLock<Mutex<HashMap<String, u16>>> = OnceLock::new();
    PROXIES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Listen on loopback and forward every connection over a pinned TLS session to `host:port`.
/// One proxy per (host, port, pin); asking again returns the running one.
pub async fn open_proxy(host: String, port: u16, expected: String) -> Result<u16, String> {
    let expected = normalize_fingerprint(&expected).ok_or("malformed fingerprint")?;
    let key = format!("{host}:{port}:{expected}");
    if let Some(local) = proxies().lock().map_err(|e| e.to_string())?.get(&key) {
        return Ok(*local);
    }
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|e| e.to_string())?;
    let local = listener.local_addr().map_err(|e| e.to_string())?.port();
    proxies()
        .lock()
        .map_err(|e| e.to_string())?
        .insert(key, local);
    tauri::async_runtime::spawn(async move {
        while let Ok((mut inbound, _)) = listener.accept().await {
            let (host, expected) = (host.clone(), expected.clone());
            tauri::async_runtime::spawn(async move {
                if let Ok(mut upstream) = tls_connect(&host, port, expected).await {
                    let _ = copy_bidirectional(&mut inbound, &mut upstream).await;
                }
            });
        }
    });
    Ok(local)
}

#[tauri::command]
pub async fn pinned_probe(host: String, port: u16, fingerprint: String) -> Result<String, String> {
    match probe(&host, port, &fingerprint).await {
        PinOutcome::Match => Ok("match".into()),
        PinOutcome::Mismatch => Ok("mismatch".into()),
        PinOutcome::Unreachable(message) => Err(message),
    }
}

#[tauri::command]
pub async fn pinned_proxy(host: String, port: u16, fingerprint: String) -> Result<u16, String> {
    open_proxy(host, port, fingerprint).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_rustls::rustls::pki_types::PrivateKeyDer;
    use tokio_rustls::rustls::ServerConfig;
    use tokio_rustls::TlsAcceptor;

    fn self_signed() -> (Vec<u8>, Vec<u8>) {
        let rcgen::CertifiedKey { cert, signing_key } =
            rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_string()]).unwrap();
        (cert.der().to_vec(), signing_key.serialize_der())
    }

    async fn serve_once() -> (u16, String) {
        let (cert, key) = self_signed();
        let fingerprint = leaf_fingerprint(&cert);
        let provider = Arc::new(tokio_rustls::rustls::crypto::ring::default_provider());
        let config = ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(cert)],
                PrivateKeyDer::try_from(key).unwrap(),
            )
            .unwrap();
        let acceptor = TlsAcceptor::from(Arc::new(config));
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                let acceptor = acceptor.clone();
                tokio::spawn(async move {
                    if let Ok(mut tls) = acceptor.accept(tcp).await {
                        let mut buf = [0u8; 4];
                        if tls.read_exact(&mut buf).await.is_ok() {
                            let _ = tls.write_all(b"pong").await;
                        }
                    }
                });
            }
        });
        (port, fingerprint)
    }

    #[tokio::test]
    async fn probe_accepts_the_pinned_leaf_and_rejects_any_other() {
        let (port, fingerprint) = serve_once().await;
        assert_eq!(
            probe("127.0.0.1", port, &fingerprint).await,
            PinOutcome::Match
        );
        let other = format!("sha256:{}", "00".repeat(32));
        assert_eq!(probe("127.0.0.1", port, &other).await, PinOutcome::Mismatch);
    }

    #[tokio::test]
    async fn proxy_forwards_bytes_only_over_the_pinned_session() {
        let (port, fingerprint) = serve_once().await;
        let local = open_proxy("127.0.0.1".into(), port, fingerprint)
            .await
            .unwrap();
        let mut client = TcpStream::connect(("127.0.0.1", local)).await.unwrap();
        client.write_all(b"ping").await.unwrap();
        let mut reply = [0u8; 4];
        client.read_exact(&mut reply).await.unwrap();
        assert_eq!(&reply, b"pong");

        let wrong = format!("sha256:{}", "11".repeat(32));
        let blocked = open_proxy("127.0.0.1".into(), port, wrong).await.unwrap();
        let mut client = TcpStream::connect(("127.0.0.1", blocked)).await.unwrap();
        let _ = client.write_all(b"ping").await;
        let mut sink = Vec::new();
        let _ = client.read_to_end(&mut sink).await;
        assert!(sink.is_empty());
    }

    #[test]
    fn fingerprints_normalise_the_forms_the_daemon_prints() {
        let colon = "AB:".repeat(31) + "AB";
        assert_eq!(
            normalize_fingerprint(&colon),
            Some(format!("sha256:{}", "ab".repeat(32)))
        );
        assert_eq!(normalize_fingerprint("sha256:zz"), None);
    }
}
