//! What `forge serve` does when the session store will not open.
//!
//! A store written by a build with a newer schema, or one a migration left half-changed, used to
//! stop the daemon (and a service manager restarted it forever). Now the daemon stays up on its
//! normal port with a small recovery surface instead: every normal route answers `503` with the
//! stable code `store_recovery`, and a handful of routes let the desktop app show what is wrong
//! and offer a way out. When the store opens, the daemon carries on starting as usual.
//!
//! The routes sit under the usual token prefix, so only a paired client can use them.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use axum::extract::{Json, State};
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;
use forge_store::db_safety;

use crate::serve::{err_response, err_response_coded, json_response};

/// Stable code every client keys on to switch to the recovery page.
pub(crate) const CODE: &str = "store_recovery";

/// How the recovery listener should speak, matching what the normal daemon would have done.
pub(crate) enum Transport {
    Plain,
    Tls,
}

struct Recovery {
    db: PathBuf,
    base: String,
    failure: Mutex<String>,
    opened: Mutex<Option<forge_store::Store>>,
    done: tokio::sync::Notify,
}

#[derive(serde::Serialize)]
struct StatusBody {
    error: String,
    schema_version: i64,
    data_dir: String,
    backups: Vec<BackupBody>,
    /// Whether "open data folder" does anything useful: a headless host has no file manager.
    can_open_folder: bool,
}

#[derive(serde::Serialize)]
struct BackupBody {
    name: String,
    before_version: i64,
    bytes: u64,
    modified: i64,
}

#[derive(serde::Serialize)]
struct Outcome {
    ok: bool,
    message: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RestoreBody {
    backup: String,
}

fn status_body(recovery: &Recovery) -> StatusBody {
    StatusBody {
        error: recovery
            .failure
            .lock()
            .map(|f| f.clone())
            .unwrap_or_default(),
        schema_version: db_safety::current_schema_version(),
        data_dir: recovery
            .db
            .parent()
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
        backups: db_safety::list_backups(&recovery.db)
            .into_iter()
            .map(|b| BackupBody {
                name: b
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                before_version: b.before_version,
                bytes: b.bytes,
                modified: b.modified,
            })
            .collect(),
        can_open_folder: folder_opener().is_some(),
    }
}

fn folder_opener() -> Option<&'static str> {
    if cfg!(target_os = "macos") {
        Some("open")
    } else if cfg!(windows) {
        Some("explorer")
    } else if std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some()
    {
        Some("xdg-open")
    } else {
        None
    }
}

/// Try to open the store; on success keep the handle for the daemon and wake the server.
fn attempt_open(recovery: &Recovery) -> Outcome {
    match crate::open_store() {
        Ok(store) => {
            if let Ok(mut slot) = recovery.opened.lock() {
                *slot = Some(store);
            }
            recovery.done.notify_one();
            Outcome {
                ok: true,
                message: "The session store opened. Forge is starting.".to_string(),
            }
        }
        Err(error) => {
            let text = format!("{error:#}");
            if let Ok(mut failure) = recovery.failure.lock() {
                *failure = text.clone();
            }
            Outcome {
                ok: false,
                message: format!("The store still will not open: {text}"),
            }
        }
    }
}

async fn status(State(recovery): State<Arc<Recovery>>) -> Response {
    json_response(&status_body(&recovery))
}

async fn retry(State(recovery): State<Arc<Recovery>>) -> Response {
    let outcome = tokio::task::spawn_blocking(move || attempt_open(&recovery)).await;
    match outcome {
        Ok(outcome) => json_response(&outcome),
        Err(_) => err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "the migration attempt did not finish",
        ),
    }
}

async fn restore(State(recovery): State<Arc<Recovery>>, Json(body): Json<RestoreBody>) -> Response {
    let outcome = tokio::task::spawn_blocking(move || {
        let Some(backup) = db_safety::list_backups(&recovery.db)
            .into_iter()
            .find(|b| b.path.file_name().is_some_and(|n| *n == *body.backup))
        else {
            return Err("no such backup".to_string());
        };
        db_safety::restore_backup(&recovery.db, &backup.path).map_err(|e| e.to_string())?;
        Ok(attempt_open(&recovery))
    })
    .await;
    match outcome {
        Ok(Ok(outcome)) => json_response(&outcome),
        Ok(Err(message)) => err_response(axum::http::StatusCode::BAD_REQUEST, &message),
        Err(_) => err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "the restore did not finish",
        ),
    }
}

async fn open_folder(State(recovery): State<Arc<Recovery>>) -> Response {
    let (Some(opener), Some(dir)) = (folder_opener(), recovery.db.parent()) else {
        return err_response(
            axum::http::StatusCode::CONFLICT,
            "this host has no file manager to open the folder with",
        );
    };
    match std::process::Command::new(opener)
        .arg(dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(_) => json_response(&Outcome {
            ok: true,
            message: format!("Opened {}", dir.display()),
        }),
        Err(error) => err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            &format!("could not open the folder: {error}"),
        ),
    }
}

async fn unavailable() -> Response {
    err_response_coded(
        axum::http::StatusCode::SERVICE_UNAVAILABLE,
        CODE,
        "the session store could not be opened; recover it from the desktop app or `forge backup`",
    )
}

fn router(recovery: Arc<Recovery>) -> Router {
    let base = recovery.base.clone();
    Router::new()
        .route(&format!("{base}/api/recovery"), get(status))
        .route(&format!("{base}/api/recovery/retry"), post(retry))
        .route(&format!("{base}/api/recovery/restore"), post(restore))
        .route(
            &format!("{base}/api/recovery/open-folder"),
            post(open_folder),
        )
        .route(&format!("{base}/api/identity"), get(identity))
        .route(
            &format!("{base}/{{*rest}}"),
            get(unavailable).post(unavailable),
        )
        .with_state(recovery)
}

/// Clients ask for the identity first to learn the daemon's protocol; answer it plainly (it
/// reads nothing from the store) so a client does not mistake recovery for a wrong token.
async fn identity() -> Response {
    json_response(&serde_json::json!({
        "hostname": crate::anywhere::default_host_name(),
        "version": env!("CARGO_PKG_VERSION"),
        "protocol": crate::remote::PROTOCOL_VERSION,
        "recovery": true,
    }))
}

/// Bind the daemon's port: loopback when the exposure is local or tunnelled, otherwise every
/// interface. Non-blocking, ready for tokio.
pub(crate) fn bind(
    loopback: bool,
    port: u16,
) -> Result<(std::net::TcpListener, std::net::SocketAddr)> {
    use anyhow::Context as _;
    let ip: std::net::IpAddr = if loopback {
        std::net::Ipv4Addr::LOCALHOST.into()
    } else {
        std::net::Ipv4Addr::UNSPECIFIED.into()
    };
    let listener = std::net::TcpListener::bind((ip, port)).with_context(|| {
        format!(
            "binding {ip}:{port} — is another forge serve running? \
             (`[remote] port` or --port picks a different one)"
        )
    })?;
    let addr = listener.local_addr()?;
    listener.set_nonblocking(true)?;
    Ok((listener, addr))
}

/// Open the session store for the daemon. A store that fails SQLite's integrity check is set
/// aside and the newest readable backup put in its place (the diagnostics screen reports it);
/// one that still will not open puts the daemon into recovery mode on `listener` until it does.
/// `cert_host` is `Some` when the daemon would normally speak self-signed HTTPS.
pub(crate) async fn open_store_or_recover(
    listener: &std::net::TcpListener,
    base: &str,
    cert_host: Option<String>,
) -> Result<forge_store::Store> {
    let db = crate::store_path();
    check_integrity(&db).await;
    let failure = match crate::open_store() {
        Ok(store) => return Ok(store),
        Err(error) => format!("{error:#}"),
    };
    announce_recovering(listener, base, cert_host.as_deref());
    let (transport, tls) = match cert_host {
        None => (Transport::Plain, None),
        Some(host) => {
            let dir = forge_config::config_dir()
                .ok_or_else(|| anyhow::anyhow!("no config directory on this platform"))?;
            let cert = crate::serve_pairing::load_or_create_cert(
                &dir,
                vec![host, "localhost".to_string()],
            )?;
            (Transport::Tls, Some((cert.cert_pem, cert.key_pem)))
        }
    };
    until_store_opens(
        failure,
        db,
        base.to_string(),
        listener.try_clone()?,
        transport,
        tls,
    )
    .await
}

/// Publish the discovery file so a desktop app that has never paired can still find this daemon
/// and show the recovery screen; the normal startup rewrites it once the store opens.
fn announce_recovering(listener: &std::net::TcpListener, base: &str, lan_host: Option<&str>) {
    let Ok(addr) = listener.local_addr() else {
        return;
    };
    let (exposure, host, scheme) = match lan_host {
        Some(host) => ("lan", host.to_string(), "https"),
        None => ("local", "127.0.0.1".to_string(), "http"),
    };
    let state = crate::serve::ServeState {
        pid: std::process::id(),
        port: addr.port(),
        exposure: exposure.to_string(),
        base_url: format!("{scheme}://{host}:{}{base}", addr.port()),
        token: base.trim_start_matches('/').to_string(),
        started_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()),
    };
    if let Err(error) = crate::serve::write_state(&state) {
        eprintln!("  could not write serve-state.json: {error}");
    }
}

async fn check_integrity(db: &std::path::Path) {
    if !db.exists() {
        return;
    }
    let path = db.to_path_buf();
    let verdict = tokio::task::spawn_blocking(move || db_safety::quick_check(&path)).await;
    match verdict {
        Ok(db_safety::Integrity::Corrupt(why)) => {
            eprintln!("⚠ the session store failed its integrity check ({why}); setting it aside");
            let reason = format!("The session store failed its integrity check ({why})");
            match db_safety::quarantine_and_restore(db, &reason) {
                Ok(notice) if notice.outcome == "restored" => eprintln!(
                    "  restored the newest backup; the damaged file is kept at {}",
                    notice.quarantined
                ),
                Ok(notice) => eprintln!(
                    "  no readable backup, starting empty; the damaged file is kept at {}",
                    notice.quarantined
                ),
                Err(error) => eprintln!("  could not set it aside: {error}"),
            }
        }
        Ok(db_safety::Integrity::Unknown(why)) => {
            eprintln!("  integrity check skipped: {why}");
        }
        _ => {}
    }
}

/// Serve the recovery surface on `listener` until the store opens, then hand the opened store back.
/// `Ctrl-C` ends the daemon instead.
pub(crate) async fn until_store_opens(
    failure: String,
    db: PathBuf,
    base: String,
    listener: std::net::TcpListener,
    transport: Transport,
    tls: Option<(Vec<u8>, Vec<u8>)>,
) -> Result<forge_store::Store> {
    eprintln!(
        "⚠ the session store will not open: {failure}\n  forge serve stays up so the desktop app can \
         recover it (restore a backup, retry the migration). `forge backup list` shows the backups."
    );
    let recovery = Arc::new(Recovery {
        db,
        base,
        failure: Mutex::new(failure),
        opened: Mutex::new(None),
        done: tokio::sync::Notify::new(),
    });
    let app = router(recovery.clone());
    let finished = {
        let recovery = recovery.clone();
        async move { recovery.done.notified().await }
    };
    let serve = async {
        match (transport, tls) {
            (Transport::Tls, Some((cert, key))) => {
                let config = axum_server::tls_rustls::RustlsConfig::from_pem(cert, key).await?;
                let handle = axum_server::Handle::new();
                let stopper = handle.clone();
                tokio::spawn(async move {
                    finished.await;
                    stopper.graceful_shutdown(Some(std::time::Duration::from_secs(2)));
                });
                axum_server::from_tcp_rustls(listener, config)?
                    .handle(handle)
                    .serve(app.into_make_service())
                    .await?;
            }
            _ => {
                let listener = tokio::net::TcpListener::from_std(listener)?;
                axum::serve(listener, app)
                    .with_graceful_shutdown(finished)
                    .await?;
            }
        }
        Ok::<(), anyhow::Error>(())
    };
    tokio::select! {
        result = serve => result?,
        _ = crate::serve::shutdown_signal() => anyhow::bail!("stopped while the session store was unavailable"),
    }
    let store = recovery
        .opened
        .lock()
        .ok()
        .and_then(|mut slot| slot.take())
        .ok_or_else(|| anyhow::anyhow!("recovery ended without an open store"))?;
    Ok(store)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;

    fn rig(dir: &std::path::Path) -> Arc<Recovery> {
        Arc::new(Recovery {
            db: dir.join("forge.db"),
            base: "/tok".to_string(),
            failure: Mutex::new("schema too new".to_string()),
            opened: Mutex::new(None),
            done: tokio::sync::Notify::new(),
        })
    }

    async fn call(app: &Router, method: &str, path: &str, body: &str) -> (u16, serde_json::Value) {
        let request = axum::http::Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body.to_string()))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status().as_u16();
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
        )
    }

    #[tokio::test]
    async fn ordinary_routes_answer_with_the_recovery_code_and_status_lists_backups() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("forge.db.pre-v46.bak"), b"x").unwrap();
        let app = router(rig(dir.path()));

        let (status, body) = call(&app, "GET", "/tok/api/sessions", "").await;
        assert_eq!(status, 503);
        assert_eq!(body["code"], CODE);

        let (status, body) = call(&app, "GET", "/tok/api/recovery", "").await;
        assert_eq!(status, 200);
        assert_eq!(body["error"], "schema too new");
        assert_eq!(body["backups"][0]["name"], "forge.db.pre-v46.bak");
        assert_eq!(body["backups"][0]["before_version"], 46);

        let (status, _) = call(&app, "GET", "/wrong/api/recovery", "").await;
        assert_eq!(status, 404, "the token prefix still gates everything");
    }

    #[tokio::test]
    async fn restoring_a_backup_that_does_not_exist_is_a_bad_request() {
        let dir = tempfile::tempdir().unwrap();
        let app = router(rig(dir.path()));
        let (status, body) = call(
            &app,
            "POST",
            "/tok/api/recovery/restore",
            r#"{"backup":"../../etc/passwd"}"#,
        )
        .await;
        assert_eq!(status, 400);
        assert_eq!(body["error"], "no such backup");
    }
}
