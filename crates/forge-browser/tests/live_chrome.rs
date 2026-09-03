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
