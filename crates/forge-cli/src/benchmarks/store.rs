//! On-disk benchmark cache: the scored rows plus where they came from and when we last asked.

use std::path::Path;

use serde_json::{json, Value};

use super::{now, Row, LEGACY_CACHE_PATH};

/// Bumped when the on-disk shape changes meaning. Version 1 wrote the intelligence index into
/// `coding` for rows without a coding index, so its coding values cannot be trusted. The source,
/// ETag and timestamp fields added with the mirror are optional, so they did not need a bump.
pub(super) const CACHE_SCHEMA: i64 = 2;

/// Where the cached rows came from. A legacy cache with no `source` was always fetched with the
/// user's own key, the only path that existed then.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Source {
    OwnKey,
    Mirror,
    Snapshot,
}

impl Source {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Source::OwnKey => "own_key",
            Source::Mirror => "mirror",
            Source::Snapshot => "snapshot",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "own_key" => Some(Source::OwnKey),
            "mirror" => Some(Source::Mirror),
            "snapshot" => Some(Source::Snapshot),
            _ => None,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Source::OwnKey => "your Artificial Analysis key",
            Source::Mirror => "Forge benchmark mirror",
            Source::Snapshot => "snapshot bundled with this build",
        }
    }
}

/// The persisted cache: scored rows, a negative cache of catalog ids the feed had NO rating for
/// (so an unlisted model — e.g. a local ollama one — doesn't trigger a fetch on every run), and
/// the timing needed to decide when to ask again.
pub(super) struct Cache {
    pub rows: Vec<Row>,
    pub unrated: Vec<String>,
    /// Seconds since the data was last fetched or confirmed current.
    pub age: i64,
    pub source: Source,
    /// `(url, etag)` of the mirror response the rows came from.
    pub etag: Option<(String, String)>,
    /// When the feed itself was read from Artificial Analysis (unix seconds).
    pub data_as_of: i64,
    /// Last time we tried the network, successfully or not (unix seconds).
    pub checked_at: i64,
}

pub(super) struct Entry<'a> {
    pub rows: &'a [Row],
    pub unrated: &'a [String],
    pub source: Source,
    pub etag: Option<(&'a str, &'a str)>,
    pub data_as_of: Option<i64>,
}

pub(super) fn load_cache(path: &Path) -> Option<Cache> {
    let body = std::fs::read_to_string(path)
        .or_else(|_| std::fs::read_to_string(LEGACY_CACHE_PATH))
        .ok()?;
    let v: Value = serde_json::from_str(&body).ok()?;
    let fetched = v.get("fetched_at").and_then(Value::as_i64)?;
    let current = v.get("schema").and_then(Value::as_i64) == Some(CACHE_SCHEMA);
    let rows = v
        .get("models")
        .and_then(Value::as_array)?
        .iter()
        .filter_map(|m| {
            let name = m.get("name")?.as_str()?.to_string();
            let intel = m.get("intelligence")?.as_f64()?;
            let coding = m.get("coding").and_then(Value::as_f64);
            Some((name, intel, coding))
        })
        .collect();
    let unrated = v
        .get("unrated")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let source = v
        .get("source")
        .and_then(Value::as_str)
        .and_then(Source::parse)
        .unwrap_or(Source::OwnKey);
    let etag = match (
        v.get("etag_url").and_then(Value::as_str),
        v.get("etag").and_then(Value::as_str),
    ) {
        (Some(url), Some(tag)) => Some((url.to_string(), tag.to_string())),
        _ => None,
    };
    Some(Cache {
        rows,
        unrated,
        // An old-schema cache is served until the refetch replaces it, but always refetched.
        age: if current { now() - fetched } else { i64::MAX },
        source,
        etag,
        data_as_of: v
            .get("data_as_of")
            .and_then(Value::as_i64)
            .unwrap_or(fetched),
        checked_at: v
            .get("checked_at")
            .and_then(Value::as_i64)
            .unwrap_or(fetched),
    })
}

pub(super) fn save_cache(path: &Path, e: &Entry<'_>) {
    let models: Vec<Value> = e
        .rows
        .iter()
        .map(|(n, i, c)| json!({ "name": n, "intelligence": i, "coding": c }))
        .collect();
    let stamp = now();
    let mut doc = json!({
        "schema": CACHE_SCHEMA,
        "fetched_at": stamp,
        "checked_at": stamp,
        "data_as_of": e.data_as_of.unwrap_or(stamp),
        "source": e.source.as_str(),
        "models": models,
        "unrated": e.unrated,
    });
    if let Some((url, tag)) = e.etag {
        doc["etag_url"] = json!(url);
        doc["etag"] = json!(tag);
    }
    write(path, &doc);
}

/// Record a network attempt on the existing cache without touching its rows. `confirmed` also
/// restarts the data's age (a 304 proved it is still current) and replaces the negative cache.
pub(super) fn touch(path: &Path, confirmed: Option<&[String]>) {
    let Ok(body) = std::fs::read_to_string(path) else {
        return;
    };
    let Ok(mut doc) = serde_json::from_str::<Value>(&body) else {
        return;
    };
    let stamp = now();
    doc["checked_at"] = json!(stamp);
    if let Some(unrated) = confirmed {
        doc["fetched_at"] = json!(stamp);
        doc["unrated"] = json!(unrated);
    }
    write(path, &doc);
}

fn write(path: &Path, doc: &Value) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(bytes) = serde_json::to_vec_pretty(doc) {
        let _ = std::fs::write(path, bytes);
    }
}
