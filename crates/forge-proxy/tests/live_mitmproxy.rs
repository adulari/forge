//! End-to-end against a REAL mitmdump. `#[ignore]`d: they need mitmproxy installed and bind a
//! port, so they run on request (`cargo test -p forge-agent-proxy -- --ignored`) rather than in
//! every CI job.

use forge_proxy::{Filter, InterceptRules, Proxy};

async fn proxy(dir: &tempfile::TempDir, port: u16) -> Proxy {
    Proxy::start(Some(port), dir.path())
        .await
        .expect("mitmdump must start")
}

/// The whole point of the crate in one test: traffic through the proxy is captured, and the
/// capture is queryable by URL, method, and body.
#[tokio::test]
#[ignore]
async fn traffic_through_the_proxy_is_captured_and_queryable() {
    let dir = tempfile::tempdir().unwrap();
    let p = proxy(&dir, 18081).await;

    let client = reqwest::Client::builder()
        .proxy(reqwest::Proxy::all("http://127.0.0.1:18081").unwrap())
        .build()
        .unwrap();
    client
        .post("http://example.com/v1/login")
        .body("{\"user\":\"floris\"}")
        .send()
        .await
        .expect("request through the proxy");

    // The addon writes on response completion, so give it a beat.
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;

    let flows = p.flows(&Filter::default(), 50).unwrap();
    assert!(!flows.is_empty(), "the proxy captured nothing");
    let login = p
        .flows(
            &Filter {
                url_contains: Some("login".into()),
                with_body: true,
                ..Default::default()
            },
            50,
        )
        .unwrap();
    assert_eq!(login.len(), 1, "the POST must be findable by URL + body");
    assert!(login[0].request_body.contains("floris"), "{:?}", login[0]);
}

/// Blocking is the "what does the app do without this endpoint" primitive, and it has to take
/// effect WITHOUT restarting the proxy — the interesting flow is often one you get once.
#[tokio::test]
#[ignore]
async fn a_block_rule_takes_effect_without_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let mut p = proxy(&dir, 18082).await;
    let client = reqwest::Client::builder()
        .proxy(reqwest::Proxy::all("http://127.0.0.1:18082").unwrap())
        .build()
        .unwrap();

    p.set_rules(InterceptRules {
        block: vec!["/telemetry".into()],
        ..Default::default()
    })
    .unwrap();

    let blocked = client
        .get("http://example.com/telemetry/event")
        .send()
        .await
        .unwrap();
    assert_eq!(
        blocked.status().as_u16(),
        418,
        "the block rule did not fire"
    );
    assert!(blocked.headers().contains_key("x-forge-blocked"));
}

/// A stub answers "how does the app behave when the server says X" without owning the server.
#[tokio::test]
#[ignore]
async fn a_stub_replaces_the_response_the_client_sees() {
    let dir = tempfile::tempdir().unwrap();
    let mut p = proxy(&dir, 18083).await;
    let client = reqwest::Client::builder()
        .proxy(reqwest::Proxy::all("http://127.0.0.1:18083").unwrap())
        .build()
        .unwrap();

    p.set_rules(InterceptRules {
        stub_response: vec![forge_proxy::StubRule {
            url_contains: "/v1/me".into(),
            status: 402,
            body: "{\"plan\":\"expired\"}".into(),
            headers: Default::default(),
        }],
        ..Default::default()
    })
    .unwrap();

    let stubbed = client.get("http://example.com/v1/me").send().await.unwrap();
    assert_eq!(stubbed.status().as_u16(), 402);
    assert!(stubbed.text().await.unwrap().contains("expired"));
}
