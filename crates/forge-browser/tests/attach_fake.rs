//! Attach mode against a fake DevTools endpoint: endpoint discovery over HTTP, and the guarantee
//! that Forge never closes or adopts anything but the one tab it opened.

use std::sync::{Arc, Mutex};

use forge_browser::attach::{discover, open_own_tab};
use forge_browser::{BrowserSession, Fingerprint};
use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[derive(Default)]
struct Log {
    http: Vec<String>,
    cdp: Vec<String>,
}

/// A fake browser: `/json/version`, `PUT /json/new`, `/json/close/<id>` over HTTP and a WebSocket
/// that answers every command with an empty result while recording its method.
async fn fake_browser() -> (String, Arc<Mutex<Log>>) {
    let log = Arc::new(Mutex::new(Log::default()));
    let ws_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_port = ws_listener.local_addr().unwrap().port();
    let ws_log = Arc::clone(&log);
    tokio::spawn(async move {
        while let Ok((stream, _)) = ws_listener.accept().await {
            let log = Arc::clone(&ws_log);
            tokio::spawn(async move {
                let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await else {
                    return;
                };
                while let Some(Ok(message)) = ws.next().await {
                    let Ok(text) = message.into_text() else {
                        continue;
                    };
                    let Ok(command) = serde_json::from_str::<Value>(text.as_str()) else {
                        continue;
                    };
                    log.lock()
                        .unwrap()
                        .cdp
                        .push(command["method"].as_str().unwrap_or_default().to_string());
                    let reply = json!({"id": command["id"], "result": {}}).to_string();
                    if ws.send(reply.into()).await.is_err() {
                        return;
                    }
                }
            });
        }
    });

    let http_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let http_port = http_listener.local_addr().unwrap().port();
    let http_log = Arc::clone(&log);
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = http_listener.accept().await {
            let log = Arc::clone(&http_log);
            tokio::spawn(async move {
                let mut buffer = [0u8; 4096];
                let read = stream.read(&mut buffer).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..read]).to_string();
                let line = request.lines().next().unwrap_or_default();
                let mut parts = line.split(' ');
                let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
                log.lock().unwrap().http.push(format!("{method} {path}"));
                let body = if path == "/json/version" {
                    json!({
                        "Browser": "Chrome/152.0.0.0",
                        "webSocketDebuggerUrl":
                            format!("ws://127.0.0.1:{ws_port}/devtools/browser/fake")
                    })
                } else if path.starts_with("/json/new") {
                    json!({
                        "id": "forge-tab",
                        "type": "page",
                        "webSocketDebuggerUrl":
                            format!("ws://127.0.0.1:{ws_port}/devtools/page/forge-tab")
                    })
                } else {
                    json!("Target is closing")
                }
                .to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
                     Connection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });
    (format!("http://127.0.0.1:{http_port}"), log)
}

#[tokio::test]
async fn discovery_reads_the_browser_websocket_from_json_version() {
    let (base, _log) = fake_browser().await;
    let endpoint = discover(&format!("{base}/anything"))
        .await
        .expect("discover");
    assert_eq!(endpoint.http_base, base);
    assert!(endpoint.browser_ws_url.ends_with("/devtools/browser/fake"));
    assert_eq!(endpoint.browser, "Chrome/152.0.0.0");
}

#[tokio::test]
async fn discovery_of_a_dead_port_explains_how_to_start_a_browser() {
    let err = discover("http://127.0.0.1:1")
        .await
        .expect_err("nothing listens");
    assert!(
        format!("{err:#}").contains("forge browser attach"),
        "{err:#}"
    );
}

#[tokio::test]
async fn attach_opens_its_own_tab_and_closes_only_that_one() {
    let (base, log) = fake_browser().await;
    let session = BrowserSession::connect(&base, &Fingerprint::default())
        .await
        .expect("connect");
    assert!(session.is_attached());
    session.eval("1 + 1").await.expect("eval");
    drop(session);
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let log = log.lock().unwrap();
    assert!(
        log.http.iter().any(|r| r == "GET /json/version"),
        "{:?}",
        log.http
    );
    assert!(
        log.http.iter().any(|r| r.starts_with("PUT /json/new")),
        "must open its own tab: {:?}",
        log.http
    );
    assert!(
        log.http.iter().any(|r| r == "GET /json/close/forge-tab"),
        "must close its own tab: {:?}",
        log.http
    );
    assert!(
        !log.http.iter().any(|r| r.starts_with("GET /json/list")),
        "attach mode must not adopt the user's tabs: {:?}",
        log.http
    );
    let closes: Vec<_> = log
        .http
        .iter()
        .filter(|r| r.contains("/json/close/"))
        .collect();
    assert_eq!(closes.len(), 1, "{closes:?}");
    for method in &log.cdp {
        assert!(
            !matches!(method.as_str(), "Browser.close" | "Target.closeTarget")
                && !method.starts_with("Target."),
            "attach mode sent {method}: {:?}",
            log.cdp
        );
    }
    assert!(
        log.cdp.iter().any(|m| m == "Runtime.evaluate"),
        "{:?}",
        log.cdp
    );
}

#[tokio::test]
async fn opening_a_tab_returns_its_id_and_endpoint() {
    let (base, _log) = fake_browser().await;
    let tab = open_own_tab(&base).await.expect("tab");
    assert_eq!(tab.id, "forge-tab");
    assert!(tab.ws_url.ends_with("/devtools/page/forge-tab"));
}
