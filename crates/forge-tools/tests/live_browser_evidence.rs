//! Browser screenshots land in the session's evidence ledger, end to end against a real Chrome.
//!
//! Ignored by default (CI has no browser). Run with one present:
//!
//! ```text
//! cargo test -p forge-agent-tools --test live_browser_evidence -- --ignored --nocapture
//! ```

use std::io::{Read, Write};
use std::net::TcpListener;

use forge_tools::evidence::{EvidenceKind, Ledger};
use forge_tools::{scope_session, ToolRegistry};
use serde_json::json;

fn serve_page() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
    let port = listener.local_addr().expect("addr").port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buffer = [0u8; 4096];
            let _ = stream.read(&mut buffer);
            let body = "<!doctype html><html><body><h1>evidence fixture</h1></body></html>";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\
                 Connection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    format!("http://127.0.0.1:{port}/page?token=secret")
}

#[tokio::test]
#[ignore = "needs a real Chrome installed"]
async fn two_screenshots_in_one_session_both_land_in_the_ledger() {
    let workspace = tempfile::tempdir().expect("workspace");
    let profiles = tempfile::tempdir().expect("profiles");
    std::env::set_var("FORGE_BROWSER_PROFILE_ROOT", profiles.path());
    let registry = ToolRegistry::with_core_tools_in(workspace.path());
    let browser = registry.get("browser").expect("browser tool");
    let url = serve_page();

    let run = |args: serde_json::Value| {
        scope_session("live-evidence".to_string(), async move {
            browser.run(&args).await
        })
    };
    run(json!({"action": "open", "url": url, "headless": true, "profile": "evidence"}))
        .await
        .expect("open");
    run(json!({"action": "screenshot", "profile": "evidence", "caption": "first"}))
        .await
        .expect("first screenshot");
    run(json!({"action": "screenshot", "profile": "evidence"}))
        .await
        .expect("second screenshot");
    run(json!({"action": "close", "profile": "evidence"}))
        .await
        .expect("close");

    let root = workspace
        .path()
        .canonicalize()
        .expect("canonical workspace");
    let ledger = Ledger::new(&root, "live-evidence").expect("ledger");
    let items = ledger.items();
    assert_eq!(items.len(), 2, "{items:?}");
    assert_eq!(items[0].file, "0001-browser.png");
    assert_eq!(items[1].file, "0002-browser.png");
    assert_eq!(items[0].caption.as_deref(), Some("first"));
    for item in &items {
        assert_eq!(item.kind, EvidenceKind::Screenshot);
        let url = item.url.as_deref().expect("page url recorded");
        assert!(url.ends_with("/page"), "{url}");
        let bytes = std::fs::read(ledger.dir().join(&item.file)).expect("screenshot file");
        assert!(bytes.starts_with(b"\x89PNG"), "not a PNG");
    }
    let block = forge_tools::evidence::pr_evidence_block(&root, "live-evidence").expect("block");
    assert!(block.contains("0001-browser.png") && block.contains("0002-browser.png"));
}
