//! End-to-end against a real Chrome.
//!
//! Ignored by default: CI runners have no browser, and a test that silently passes when Chrome is
//! absent would assert nothing. Run with a browser present:
//!
//! ```text
//! cargo test -p forge-agent-browser --test live_chrome -- --ignored --nocapture
//! ```
//!
//! Everything here is served from a local socket, so the test needs no network and cannot depend
//! on a third party staying up.

use std::io::{Read, Write};
use std::net::TcpListener;

use forge_browser::{BrowserSession, Filter, LaunchConfig};

/// A one-shot HTTP server that serves a page which immediately POSTs, so the test can assert on a
/// request the page made on its own — the case injected `fetch` hooks are used for, and the case
/// this crate exists to cover properly.
fn serve_fixture() -> (String, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
    let port = listener.local_addr().expect("addr").port();
    let handle = std::thread::spawn(move || {
        for stream in listener.incoming().take(2) {
            let Ok(mut stream) = stream else { continue };
            let mut buffer = [0u8; 4096];
            let read = stream.read(&mut buffer).unwrap_or(0);
            let request = String::from_utf8_lossy(&buffer[..read]).to_string();
            let body = if request.starts_with("POST /api/login") {
                "{\"token\":\"tok_live_12345\"}"
            } else {
                "<!doctype html><html><body><h1 id=t>fixture</h1><script>\
                 fetch('/api/login', {method:'POST', headers:{'content-type':'application/json'},\
                 body: JSON.stringify({user:'u', pass:'p'})});</script></body></html>"
            };
            let content_type = if request.starts_with("POST") {
                "application/json"
            } else {
                "text/html"
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\
                 Connection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    (format!("http://127.0.0.1:{port}/"), handle)
}

#[tokio::test]
#[ignore = "needs a real Chrome installed"]
async fn a_real_browser_loads_a_page_and_its_traffic_is_captured() {
    let profile = tempfile::tempdir().expect("temp profile");
    let (url, server) = serve_fixture();

    let config = LaunchConfig::new(profile.path()).headless(true);
    let session = BrowserSession::open(&config)
        .await
        .expect("launch a real Chrome");

    let landed = session.navigate(&url).await.expect("navigate");
    assert!(
        landed.starts_with("http://127.0.0.1:"),
        "landed on {landed}"
    );

    // The page rendered, and we read it AFTER its script ran — the thing web_fetch cannot do.
    let html = session.html().await.expect("read html");
    assert!(html.contains("fixture"), "{html}");

    let title = session.eval("document.title").await.expect("eval");
    assert!(title.is_string(), "eval returns a value: {title:?}");

    // Wait for the page's own POST to COMPLETE, not merely to be issued. A body only exists once
    // the response has finished — waiting on the request alone raced, and `response_body`
    // correctly refused rather than handing back an empty string that would read as
    // "the server returned nothing".
    for _ in 0..60 {
        let seen = session.network(&Filter {
            url_contains: Some("/api/login".into()),
            ..Filter::default()
        });
        if seen.iter().any(|exchange| exchange.finished) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    let posts = session.network(&Filter {
        url_contains: Some("/api/login".into()),
        method: Some("POST".into()),
        ..Filter::default()
    });
    assert_eq!(
        posts.len(),
        1,
        "the page's own POST was captured: {posts:?}"
    );
    let post = &posts[0];
    assert_eq!(post.status, Some(200));
    assert!(
        post.post_data
            .as_deref()
            .is_some_and(|b| b.contains("pass")),
        "the request body is captured: {:?}",
        post.post_data
    );
    assert!(
        post.request_headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("content-type")),
        "request headers are captured: {:?}",
        post.request_headers
    );

    // And the response body, which is where a token would be.
    let body = session
        .response_body(&post.id)
        .await
        .expect("fetch the response body");
    assert!(body.contains("tok_live_12345"), "{body}");

    // The document load is captured too — not just what page script chose to send.
    let documents = session.network(&Filter {
        resource_type: Some("Document".into()),
        ..Filter::default()
    });
    assert!(
        !documents.is_empty(),
        "the navigation itself is in the log, which a fetch hook never sees"
    );

    drop(session);
    let _ = server.join();
}

#[tokio::test]
#[ignore = "needs a real Chrome installed"]
async fn a_second_session_reattaches_to_the_same_browser() {
    // The behaviour that makes a hand-performed login worth anything: the browser outlives the
    // session that opened it, and the next turn drives the same window.
    let profile = tempfile::tempdir().expect("temp profile");
    let config = LaunchConfig::new(profile.path()).headless(true);

    let mut first = BrowserSession::open(&config).await.expect("launch");
    first.detach();
    let port_before = format!("{first:?}");
    drop(first);

    let second = BrowserSession::reattach(profile.path())
        .await
        .expect("re-attach to the browser left running");
    let port_after = format!("{second:?}");
    assert_eq!(
        port_before.split("port:").nth(1),
        port_after.split("port:").nth(1),
        "re-attach must reach the SAME browser, not launch a second one"
    );
}

/// A fixture that stays up for the whole test and echoes each request's method, path, headers and
/// body as JSON. Used to prove that a replayed or intercepted request reached the server changed.
fn serve_echo() -> (
    String,
    std::sync::mpsc::Sender<()>,
    std::thread::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind echo server");
    listener.set_nonblocking(true).expect("nonblocking");
    let port = listener.local_addr().expect("addr").port();
    let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
    let handle = std::thread::spawn(move || loop {
        if stop_rx.try_recv().is_ok() {
            break;
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                let mut buffer = [0u8; 8192];
                let read = stream.read(&mut buffer).unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..read]).to_string();
                let first_line = request.lines().next().unwrap_or("");
                let mut parts = first_line.split_whitespace();
                let method = parts.next().unwrap_or("");
                let path = parts.next().unwrap_or("");
                let body = request.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("");
                let has_debug = request.to_lowercase().contains("x-debug:");
                let json = format!(
                    "{{\"method\":\"{method}\",\"path\":\"{path}\",\"body\":{},\"sawDebug\":{}}}",
                    serde_json::to_string(body).unwrap_or_else(|_| "\"\"".into()),
                    has_debug
                );
                let is_page = method == "GET" && path == "/";
                let payload = if is_page {
                    "<!doctype html><html><body>fixture<script>\
                     fetch('/noise/analytics'); fetch('/api/echo', {method:'POST', body:'orig'});\
                     </script></body></html>"
                        .to_string()
                } else {
                    json
                };
                let content_type = if is_page {
                    "text/html"
                } else {
                    "application/json"
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nAccess-Control-Allow-Origin: *\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                    payload.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Err(_) => break,
        }
    });
    (format!("http://127.0.0.1:{port}"), stop_tx, handle)
}

#[tokio::test]
#[ignore = "needs a real Chrome installed"]
async fn replay_reissues_a_request_with_changes_and_the_server_sees_them() {
    let profile = tempfile::tempdir().expect("temp profile");
    let (base, stop, server) = serve_echo();
    let session = BrowserSession::open(&LaunchConfig::new(profile.path()).headless(true))
        .await
        .expect("launch");
    session
        .navigate(&format!("{base}/"))
        .await
        .expect("navigate");

    // Re-issue the page's POST with a changed body and an added header, reusing the page origin.
    let response = session
        .replay(&forge_browser::ReplayRequest {
            method: "POST".into(),
            url: format!("{base}/api/echo"),
            headers: vec![("X-Debug".into(), "1".into())],
            body: Some("CHANGED".into()),
        })
        .await
        .expect("replay");

    assert_eq!(response["status"], 200, "{response}");
    let echoed: serde_json::Value =
        serde_json::from_str(response["body"].as_str().unwrap_or("{}")).expect("echo json");
    assert_eq!(echoed["method"], "POST");
    assert_eq!(
        echoed["body"], "CHANGED",
        "the changed body reached the server"
    );
    assert_eq!(
        echoed["sawDebug"], true,
        "the added header reached the server"
    );

    drop(session);
    let _ = stop.send(());
    let _ = server.join();
}

#[tokio::test]
#[ignore = "needs a real Chrome installed"]
async fn interception_blocks_a_request_and_the_page_still_loads() {
    // The failure mode this guards: `Fetch.enable` pauses EVERY request, so a bug in the resolver
    // hangs the whole page. If the navigation below completes, the pump resolved every paused
    // request — and the block took effect.
    let profile = tempfile::tempdir().expect("temp profile");
    let (base, stop, server) = serve_echo();
    let session = BrowserSession::open(&LaunchConfig::new(profile.path()).headless(true))
        .await
        .expect("launch");

    session
        .set_interception(forge_browser::InterceptionRules {
            block: vec!["/noise/".into()],
            ..Default::default()
        })
        .await
        .expect("set interception");

    // Completes only if every paused request was resolved.
    session
        .navigate(&format!("{base}/"))
        .await
        .expect("navigate");

    for _ in 0..40 {
        if session
            .network(&forge_browser::Filter {
                url_contains: Some("/noise/".into()),
                ..Default::default()
            })
            .iter()
            .any(|e| e.failure.is_some())
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    let noise = session.network(&forge_browser::Filter {
        url_contains: Some("/noise/".into()),
        ..Default::default()
    });
    assert!(
        noise.iter().any(|e| e.failure.is_some()),
        "the blocked request is recorded as failed: {noise:?}"
    );

    session.clear_interception().await.expect("clear");
    drop(session);
    let _ = stop.send(());
    let _ = server.join();
}
