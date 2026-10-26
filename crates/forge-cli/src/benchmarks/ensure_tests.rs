//! Source precedence (own key, mirror, snapshot), the mirror's HTTP behaviour, and the publisher's
//! transform and validation. Network tests run against a local socket and a temp cache.

use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::mirror::{build_mirror, check_not_shrunk, export_checked, parse_mirror, snapshot};
use super::*;

type Handler = Arc<dyn Fn(&str) -> String + Send + Sync>;

struct Server {
    base: String,
    requests: Arc<Mutex<Vec<String>>>,
}

impl Server {
    fn hits(&self, path: &str) -> usize {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.starts_with(&format!("GET {path} ")))
            .count()
    }

    fn last(&self, path: &str) -> String {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .rfind(|r| r.starts_with(&format!("GET {path} ")))
            .cloned()
            .unwrap_or_default()
    }
}

fn http(status: u16, headers: &[(&str, &str)], body: &str) -> String {
    let mut out = format!(
        "HTTP/1.1 {status} X\r\ncontent-length: {}\r\nconnection: close\r\n",
        body.len()
    );
    for (k, v) in headers {
        out.push_str(&format!("{k}: {v}\r\n"));
    }
    out.push_str("\r\n");
    out.push_str(body);
    out
}

async fn serve(handler: Handler) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let log = requests.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            let (handler, log) = (handler.clone(), log.clone());
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let reply = handler(&req);
                log.lock().unwrap().push(req);
                let _ = sock.write_all(reply.as_bytes()).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    Server { base, requests }
}

const API_BODY: &str = r#"{"data":[{"name":"Own Key Model","artificial_analysis_intelligence_index":50,"artificial_analysis_coding_index":60}]}"#;

fn mirror_body(names: &[&str]) -> String {
    let rows: Vec<Row> = names
        .iter()
        .enumerate()
        .map(|(i, n)| (n.to_string(), 40.0 + i as f64, Some(55.0 + i as f64)))
        .collect();
    build_mirror(&rows, 1_700_000_000, 1_700_000_100).to_string()
}

fn site_handler(etag: &'static str, names: &'static [&'static str]) -> Handler {
    Arc::new(move |req: &str| {
        let lower = req.to_ascii_lowercase();
        if req.starts_with("GET /api ") {
            http(200, &[], API_BODY)
        } else if req.starts_with("GET /mirror ") {
            if lower.contains(&format!("if-none-match: {}", etag.to_ascii_lowercase())) {
                http(304, &[("etag", etag)], "")
            } else {
                http(200, &[("etag", etag)], &mirror_body(names))
            }
        } else {
            http(404, &[], "")
        }
    })
}

fn endpoints(dir: &tempfile::TempDir, server: &Server, key: Option<&str>) -> Endpoints {
    Endpoints {
        cache: dir.path().join("benchmarks.json"),
        api_url: format!("{}/api", server.base),
        mirror_urls: vec![format!("{}/mirror", server.base)],
        key: key.map(str::to_string),
        mirror_enabled: true,
    }
}

fn cached_source(ep: &Endpoints) -> Source {
    load_cache(&ep.cache).expect("cache written").source
}

fn age_cache(ep: &Endpoints) {
    let mut doc: Value =
        serde_json::from_str(&std::fs::read_to_string(&ep.cache).unwrap()).unwrap();
    doc["fetched_at"] = Value::from(now() - 10 * 3600);
    doc["checked_at"] = Value::from(now() - 10 * 3600);
    std::fs::write(&ep.cache, serde_json::to_vec(&doc).unwrap()).unwrap();
}

#[tokio::test]
async fn own_key_beats_the_mirror() {
    let dir = tempfile::tempdir().unwrap();
    let server = serve(site_handler("\"v1\"", &["Mirror A", "Mirror B"])).await;
    let ep = endpoints(&dir, &server, Some("secret"));
    let scores = resolve(&ep, &[], false).await.unwrap();
    assert_eq!(
        scores.len(),
        1,
        "the key's one-model feed, not the mirror's two"
    );
    assert_eq!(cached_source(&ep), Source::OwnKey);
    assert_eq!(server.hits("/mirror"), 0);
}

#[tokio::test]
async fn without_a_key_the_mirror_is_used_and_cached_with_its_etag() {
    let dir = tempfile::tempdir().unwrap();
    let server = serve(site_handler("\"v1\"", &["Mirror A", "Mirror B"])).await;
    let ep = endpoints(&dir, &server, None);
    let scores = resolve(&ep, &[], false).await.unwrap();
    assert_eq!(scores.len(), 2);
    let cache = load_cache(&ep.cache).unwrap();
    assert_eq!(cache.source, Source::Mirror);
    assert_eq!(cache.etag.unwrap().1, "\"v1\"");
    assert_eq!(cache.data_as_of, 1_700_000_000);
    assert_eq!(server.hits("/api"), 0, "no key, no direct fetch");
}

#[tokio::test]
async fn a_fresh_cache_does_not_touch_the_network() {
    let dir = tempfile::tempdir().unwrap();
    let server = serve(site_handler("\"v1\"", &["Mirror A", "Mirror B"])).await;
    let ep = endpoints(&dir, &server, None);
    resolve(&ep, &[], false).await.unwrap();
    resolve(&ep, &[], false).await.unwrap();
    assert_eq!(server.hits("/mirror"), 1);
}

#[tokio::test]
async fn a_stale_cache_revalidates_with_the_etag_and_keeps_its_rows_on_304() {
    let dir = tempfile::tempdir().unwrap();
    let server = serve(site_handler("\"v1\"", &["Mirror A", "Mirror B"])).await;
    let ep = endpoints(&dir, &server, None);
    resolve(&ep, &[], false).await.unwrap();
    age_cache(&ep);
    assert!(load_cache(&ep.cache).unwrap().age > MIRROR_TTL_SECS);

    let scores = resolve(&ep, &[], false).await.unwrap();
    assert_eq!(scores.len(), 2);
    assert_eq!(server.hits("/mirror"), 2);
    assert!(
        server
            .last("/mirror")
            .to_ascii_lowercase()
            .contains("if-none-match: \"v1\""),
        "second request must be conditional"
    );
    let cache = load_cache(&ep.cache).unwrap();
    assert_eq!(cache.source, Source::Mirror);
    assert!(cache.age < 60, "a 304 restarts the data's age");
}

#[tokio::test]
async fn force_skips_the_conditional_header() {
    let dir = tempfile::tempdir().unwrap();
    let server = serve(site_handler("\"v1\"", &["Mirror A", "Mirror B"])).await;
    let ep = endpoints(&dir, &server, None);
    resolve(&ep, &[], false).await.unwrap();
    resolve(&ep, &[], true).await.unwrap();
    assert!(!server
        .last("/mirror")
        .to_ascii_lowercase()
        .contains("if-none-match"));
}

#[tokio::test]
async fn an_unrated_catalog_model_refetches_but_at_most_hourly() {
    let dir = tempfile::tempdir().unwrap();
    let server = serve(site_handler("\"v1\"", &["Mirror A", "Mirror B"])).await;
    let ep = endpoints(&dir, &server, None);
    resolve(&ep, &[], false).await.unwrap();
    let ids = vec!["anthropic::brand-new-model".to_string()];

    // Fetched moments ago: the new model alone must not trigger another request.
    resolve(&ep, &ids, false).await.unwrap();
    assert_eq!(server.hits("/mirror"), 1);

    let mut doc: Value =
        serde_json::from_str(&std::fs::read_to_string(&ep.cache).unwrap()).unwrap();
    doc["checked_at"] = Value::from(now() - 2 * 3600);
    std::fs::write(&ep.cache, serde_json::to_vec(&doc).unwrap()).unwrap();
    resolve(&ep, &ids, false).await.unwrap();
    assert_eq!(server.hits("/mirror"), 2, "an hour later it asks again");

    resolve(&ep, &ids, false).await.unwrap();
    assert_eq!(server.hits("/mirror"), 2, "and the miss is remembered");
}

#[tokio::test]
async fn the_second_mirror_url_is_the_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let server = serve(site_handler("\"v1\"", &["Mirror A", "Mirror B"])).await;
    let mut ep = endpoints(&dir, &server, None);
    ep.mirror_urls.insert(0, format!("{}/gone", server.base));
    let scores = resolve(&ep, &[], false).await.unwrap();
    assert_eq!(scores.len(), 2);
    assert_eq!(server.hits("/gone"), 1);
    assert_eq!(
        load_cache(&ep.cache).unwrap().etag.unwrap().0,
        ep.mirror_urls[1]
    );
}

#[tokio::test]
async fn an_unreachable_mirror_falls_back_to_the_snapshot_once() {
    let dir = tempfile::tempdir().unwrap();
    let server = serve(Arc::new(|_: &str| http(500, &[], ""))).await;
    let ep = endpoints(&dir, &server, None);
    let scores = resolve(&ep, &[], false).await.unwrap();
    let snap = snapshot().expect("bundled snapshot parses");
    assert_eq!(scores.len(), rows_to_scores(&snap.rows).len());
    assert_eq!(cached_source(&ep), Source::Snapshot);

    resolve(&ep, &[], false).await.unwrap();
    assert_eq!(
        server.hits("/mirror"),
        1,
        "the seeded snapshot stops a retry on every run"
    );
}

#[tokio::test]
async fn opting_out_never_contacts_the_mirror() {
    let dir = tempfile::tempdir().unwrap();
    let server = serve(site_handler("\"v1\"", &["Mirror A", "Mirror B"])).await;
    let mut ep = endpoints(&dir, &server, None);
    ep.mirror_enabled = false;
    let scores = resolve(&ep, &[], true).await.unwrap();
    assert!(scores.len() > 2, "snapshot rows");
    assert_eq!(cached_source(&ep), Source::Snapshot);
    assert_eq!(server.hits("/mirror"), 0);
    assert_eq!(server.hits("/api"), 0);
}

#[tokio::test]
async fn a_rejected_key_falls_through_to_the_mirror() {
    let dir = tempfile::tempdir().unwrap();
    let server = serve(Arc::new(|req: &str| {
        if req.starts_with("GET /api ") {
            http(401, &[], "")
        } else {
            http(
                200,
                &[("etag", "\"v1\"")],
                &mirror_body(&["Mirror A", "Mirror B"]),
            )
        }
    }))
    .await;
    let ep = endpoints(&dir, &server, Some("bad"));
    let scores = resolve(&ep, &[], false).await.unwrap();
    assert_eq!(scores.len(), 2);
    assert_eq!(cached_source(&ep), Source::Mirror);
}

#[tokio::test]
async fn adding_a_key_later_replaces_a_mirror_cache() {
    let dir = tempfile::tempdir().unwrap();
    let server = serve(site_handler("\"v1\"", &["Mirror A", "Mirror B"])).await;
    let mut ep = endpoints(&dir, &server, None);
    resolve(&ep, &[], false).await.unwrap();
    age_cache(&ep);
    ep.key = Some("secret".to_string());
    let scores = resolve(&ep, &[], false).await.unwrap();
    assert_eq!(scores.len(), 1);
    assert_eq!(cached_source(&ep), Source::OwnKey);
}

#[tokio::test]
async fn ranking_disabled_means_no_data_and_no_fetch() {
    let mut config = forge_config::Config::default();
    config.mesh.benchmark_ranking = false;
    assert!(ensure(&config, &[], true).await.is_none());
}

#[test]
fn a_cache_written_before_the_mirror_reads_as_an_own_key_cache() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("benchmarks.json");
    let legacy = serde_json::json!({
        "schema": 2, "fetched_at": now() - 100,
        "models": [{"name": "Old", "intelligence": 40.0, "coding": null}],
        "unrated": [],
    });
    std::fs::write(&path, legacy.to_string()).unwrap();
    let cache = load_cache(&path).unwrap();
    assert_eq!(cache.source, Source::OwnKey);
    assert_eq!(cache.rows[0].2, None);
}

#[test]
fn the_export_estimates_missing_coding_and_says_so() {
    let rows: Vec<Row> = vec![
        ("GPT-5.6 Sol (max)".into(), 47.0, Some(77.4)),
        ("Claude Opus 5 (max)".into(), 50.8, Some(78.0)),
        ("GPT-6 Sol (max)".into(), 47.5, None),
    ];
    let doc = build_mirror(&rows, 10, 20);
    assert_eq!(doc["schema"], 1);
    assert_eq!(doc["generated_at"], 20);
    assert_eq!(doc["source_fetched_at"], 10);
    assert_eq!(
        doc["attribution"]["text"],
        "Benchmark data: Artificial Analysis"
    );
    assert_eq!(doc["attribution"]["url"], "https://artificialanalysis.ai");
    let models = doc["models"].as_array().unwrap();
    assert_eq!(
        models[0]["name"], "Claude Opus 5 (max)",
        "highest intelligence first"
    );
    let sol = models
        .iter()
        .find(|m| m["name"] == "GPT-6 Sol (max)")
        .unwrap();
    assert_eq!(sol["coding_estimated"], true);
    let coding = sol["coding"].as_f64().unwrap();
    assert!((70.0..=78.0).contains(&coding), "{coding}");
    let measured = models
        .iter()
        .find(|m| m["name"] == "GPT-5.6 Sol (max)")
        .unwrap();
    assert_eq!(measured["coding"], 77.4);
    assert_eq!(measured["coding_estimated"], false);

    let back = parse_mirror(&doc.to_string()).unwrap();
    assert_eq!(back.rows.len(), 3);
    assert!(back.rows.iter().all(|(_, _, c)| c.is_some()));
}

#[test]
fn a_malformed_or_out_of_range_mirror_is_rejected() {
    let ok = build_mirror(&[("A".into(), 10.0, Some(20.0))], 1, 1);
    assert!(parse_mirror(&ok.to_string()).is_some());

    let mut wrong_schema = ok.clone();
    wrong_schema["schema"] = 9.into();
    assert!(parse_mirror(&wrong_schema.to_string()).is_none());

    let mut empty = ok.clone();
    empty["models"] = serde_json::json!([]);
    assert!(parse_mirror(&empty.to_string()).is_none());

    let mut out_of_range = ok.clone();
    out_of_range["models"][0]["intelligence"] = 250.into();
    assert!(parse_mirror(&out_of_range.to_string()).is_none());

    assert!(parse_mirror("<html>not json</html>").is_none());
}

#[test]
fn shrinking_by_more_than_half_is_refused() {
    assert!(check_not_shrunk(100, 100).is_ok());
    assert!(check_not_shrunk(50, 100).is_ok());
    assert!(check_not_shrunk(49, 100).is_err());
    assert!(
        check_not_shrunk(5, 0).is_ok(),
        "no previous publish is no baseline"
    );

    let dir = tempfile::tempdir().unwrap();
    let previous = dir.path().join("prev.json");
    let big: Vec<Row> = (0..10)
        .map(|i| (format!("M{i}"), 10.0 + i as f64, Some(20.0)))
        .collect();
    std::fs::write(&previous, build_mirror(&big, 1, 1).to_string()).unwrap();
    let small = build_mirror(&big[..3], 2, 2);
    let err = export_checked(&small, Some(&previous))
        .unwrap_err()
        .to_string();
    assert!(err.contains("refusing to publish"), "{err}");
    assert_eq!(
        export_checked(&build_mirror(&big, 2, 2), Some(&previous)).unwrap(),
        10
    );

    std::fs::write(&previous, "garbage").unwrap();
    assert!(export_checked(&small, Some(&previous)).is_ok());
    assert!(export_checked(&small, Some(&dir.path().join("missing.json"))).is_ok());
    assert!(export_checked(&build_mirror(&[], 1, 1), None).is_err());
}

#[test]
fn the_bundled_snapshot_is_a_valid_mirror_document() {
    let snap = snapshot().expect("snapshot parses and validates");
    assert!(snap.rows.len() > 50, "{}", snap.rows.len());
    assert!(snap.source_fetched_at > 1_700_000_000);
}
