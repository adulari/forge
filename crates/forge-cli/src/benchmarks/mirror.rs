//! The Forge benchmark mirror: the published, keyless copy of the Artificial Analysis indices.
//!
//! A scheduled workflow runs `forge benchmarks --export-mirror` with the maintainers' key and
//! publishes the result; clients without a key read it here. The same document shape is bundled
//! into the binary as the last-resort snapshot.

use std::path::Path;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use super::{now, resolve_coding, Row};

pub(crate) const ATTRIBUTION_LINE: &str =
    "Benchmark data: Artificial Analysis (artificialanalysis.ai)";
const ATTRIBUTION_TEXT: &str = "Benchmark data: Artificial Analysis";
const ATTRIBUTION_URL: &str = "https://artificialanalysis.ai";

pub(super) const MIRROR_SCHEMA: i64 = 1;
pub(super) const SITE_URL: &str = "https://forge.adulari.dev/data/benchmarks.json";
pub(super) const RELEASE_URL: &str =
    "https://github.com/adulari/forge/releases/download/benchmarks-data/benchmarks.json";
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// Refresh with `scripts/refresh-benchmarks-snapshot.sh` when preparing a release.
const SNAPSHOT: &str = include_str!("../../data/benchmarks-snapshot.json");

/// A mirror document read back into rows. Coding is always present: the publisher estimates the
/// indices Artificial Analysis has not published yet and flags them `coding_estimated`.
pub(super) struct Mirror {
    pub source_fetched_at: i64,
    pub rows: Vec<Row>,
}

pub(super) fn snapshot() -> Option<Mirror> {
    parse_mirror(SNAPSHOT)
}

pub(super) fn parse_mirror(body: &str) -> Option<Mirror> {
    let v: Value = serde_json::from_str(body).ok()?;
    if v.get("schema").and_then(Value::as_i64) != Some(MIRROR_SCHEMA) {
        return None;
    }
    let rows: Vec<Row> = v
        .get("models")?
        .as_array()?
        .iter()
        .filter_map(|m| {
            Some((
                m.get("name")?.as_str()?.to_string(),
                m.get("intelligence")?.as_f64()?,
                Some(m.get("coding")?.as_f64()?),
            ))
        })
        .collect();
    validate(&rows).ok()?;
    Some(Mirror {
        source_fetched_at: v.get("source_fetched_at").and_then(Value::as_i64)?,
        rows,
    })
}

/// A feed that is empty, non-finite or off the 0–100 scale is a broken publish, not data to rank on.
pub(super) fn validate(rows: &[Row]) -> Result<(), String> {
    if rows.is_empty() {
        return Err("no models".into());
    }
    for (name, intel, coding) in rows {
        for x in std::iter::once(*intel).chain(*coding) {
            if !x.is_finite() || !(0.0..=100.0).contains(&x) {
                return Err(format!("{name}: index {x} outside 0-100"));
            }
        }
    }
    Ok(())
}

/// A publish that lost more than half of the previous models is a truncated upstream response.
pub(super) fn check_not_shrunk(new: usize, previous: usize) -> Result<(), String> {
    if previous > 0 && new * 2 < previous {
        return Err(format!(
            "model count fell from {previous} to {new} (more than 50%)"
        ));
    }
    Ok(())
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

pub(super) fn build_mirror(rows: &[Row], source_fetched_at: i64, generated_at: i64) -> Value {
    let mut models = resolve_coding(rows);
    models.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let models: Vec<Value> = models
        .into_iter()
        .map(|(name, intelligence, coding, estimated)| {
            json!({
                "name": name,
                "intelligence": round2(intelligence),
                "coding": round2(coding),
                "coding_estimated": estimated,
            })
        })
        .collect();
    json!({
        "schema": MIRROR_SCHEMA,
        "generated_at": generated_at,
        "source_fetched_at": source_fetched_at,
        "attribution": { "text": ATTRIBUTION_TEXT, "url": ATTRIBUTION_URL },
        "models": models,
    })
}

pub(super) enum Fetched {
    Fresh {
        mirror: Mirror,
        url: String,
        etag: Option<String>,
    },
    NotModified,
}

/// Try each URL in turn; the first that answers with a valid document (or 304) wins. `etag` is
/// sent only to the URL it was issued by, since the two hosts do not share validators.
pub(super) async fn fetch_mirror(
    urls: &[String],
    etag: Option<&(String, String)>,
) -> Option<Fetched> {
    let client = forge_provider::bundled_http_client();
    for url in urls {
        let mut req = client.get(url).timeout(FETCH_TIMEOUT);
        if let Some((tag_url, tag)) = etag {
            if tag_url == url {
                req = req.header(reqwest::header::IF_NONE_MATCH, tag);
            }
        }
        let Ok(resp) = req.send().await else {
            tracing::debug!("benchmark mirror unreachable");
            continue;
        };
        if resp.status() == reqwest::StatusCode::NOT_MODIFIED {
            return Some(Fetched::NotModified);
        }
        if !resp.status().is_success() {
            tracing::debug!("benchmark mirror returned {}", resp.status());
            continue;
        }
        let etag = resp
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let Ok(body) = resp.text().await else {
            continue;
        };
        if let Some(mirror) = parse_mirror(&body) {
            return Some(Fetched::Fresh {
                mirror,
                url: url.clone(),
                etag,
            });
        }
        tracing::debug!("benchmark mirror body rejected");
    }
    None
}

/// `forge benchmarks --export-mirror PATH [--previous PATH]`: fetch the feed with the key in the
/// environment, transform it to the published schema, validate it, and write it. Fails without
/// writing when the result is not safe to publish.
pub(crate) async fn export(path: &Path, previous: Option<&Path>) -> Result<()> {
    let Some(key) = forge_config::benchmark_api_key() else {
        bail!("ARTIFICIALANALYSIS_API_KEY is not set");
    };
    let Some(rows) = super::fetch_api(super::API_URL, &key).await else {
        bail!("the Artificial Analysis API returned no usable models");
    };
    let doc = build_mirror(&rows, now(), now());
    let published = export_checked(&doc, previous)?;
    std::fs::write(path, serde_json::to_vec_pretty(&doc)?)
        .with_context(|| format!("writing {}", path.display()))?;
    println!("exported {published} models to {}", path.display());
    Ok(())
}

/// Validate a built document (and its size against the previous publish); returns the model count.
pub(super) fn export_checked(doc: &Value, previous: Option<&Path>) -> Result<usize> {
    let parsed = parse_mirror(&doc.to_string()).context("exported document failed validation")?;
    if let Some(prev) = previous.filter(|p| p.exists()) {
        let body =
            std::fs::read_to_string(prev).with_context(|| format!("reading {}", prev.display()))?;
        // An unreadable previous publish is no baseline, not a reason to block publishing.
        if let Some(old) = parse_mirror(&body) {
            if let Err(e) = check_not_shrunk(parsed.rows.len(), old.rows.len()) {
                bail!("refusing to publish: {e}");
            }
        }
    }
    Ok(parsed.rows.len())
}
