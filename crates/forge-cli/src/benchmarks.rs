//! Measured model performance from Artificial Analysis (ADR-0011), as a
//! [`forge_mesh::BenchmarkScores`] for the mesh to rank on. Network + disk live here (the binary);
//! `forge-mesh` stays pure. Sources, in precedence order: the user's own Artificial Analysis key,
//! the keyless Forge mirror, the snapshot bundled into the binary. Best-effort throughout: when
//! none yields data the mesh falls back to its family-name heuristic.

mod mirror;
mod store;

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use forge_mesh::BenchmarkScores;
use serde_json::Value;

pub(crate) use mirror::{export as export_mirror, ATTRIBUTION_LINE};
use store::{load_cache, save_cache, touch, Cache, Entry, Source};

/// Legacy project-local cache path (pre-1.2). Still READ as a fallback so an existing project's
/// scores aren't lost on upgrade; new writes go to the global path.
const LEGACY_CACHE_PATH: &str = ".forge/benchmarks.json";

/// The benchmark cache lives in the GLOBAL data dir (`~/.local/share/forge/benchmarks.json`), not
/// per-project — AA scores are model-wide, so every project should share one cache and one refresh
/// (a project-local file re-fetched on every new repo and could go stale independently).
fn cache_path() -> PathBuf {
    forge_config::data_dir()
        .map(|d| d.join("benchmarks.json"))
        .unwrap_or_else(|| PathBuf::from(LEGACY_CACHE_PATH))
}
/// Safety backstop only: scores move slowly, so we normally DON'T re-fetch — the trigger is a new
/// catalog model with no rating yet. This long TTL just guarantees the dataset can't go infinitely
/// stale even if the model set never changes.
const SAFETY_TTL_SECS: i64 = 30 * 24 * 3600;
/// A cache holding rows whose coding index AA has not published yet is re-fetched daily, so a
/// model's real coding score replaces the estimate soon after it lands.
const PARTIAL_TTL_SECS: i64 = 24 * 3600;
/// The mirror is republished every three hours; clients ask at most this often (a 304 is cheap).
const MIRROR_TTL_SECS: i64 = 6 * 3600;
/// A model the cache cannot score, or a failed attempt, is retried at most this often.
const RETRY_SECS: i64 = 3600;
const API_URL: &str = "https://artificialanalysis.ai/api/v2/data/llms/models";

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs() as i64
}

/// One parsed row: (source model name/slug, intelligence index, coding index), each 0–100. The
/// coding index is `None` where the source has not published it.
type Row = (String, f64, Option<f64>);

/// Recursively find a numeric field by key anywhere within a model entry (the API nests the
/// indices under an `evaluations`-style object in some versions, flat in others).
fn find_f64(v: &Value, key: &str) -> Option<f64> {
    match v {
        Value::Object(m) => {
            if let Some(x) = m.get(key).and_then(Value::as_f64) {
                return Some(x);
            }
            m.values().find_map(|val| find_f64(val, key))
        }
        Value::Array(a) => a.iter().find_map(|e| find_f64(e, key)),
        _ => None,
    }
}

/// Parse the API body into rows. Tolerant of `{data:[…]}` vs a bare array and of where the indices
/// live within each entry. A row needs a name and an intelligence index; coding may be absent.
///
/// Scale is decided ONCE for the whole dataset, not per value: the API reports a 0–100 index
/// (Opus ≈ 56), but some versions report 0–1 fractions. A per-value `x <= 1.5 → x*100` rule
/// wrongly inflated genuinely weak models (a real 1.3 coding score became 130). So we look at the
/// max index across all rows: only if EVERYTHING is ≤ 1.5 do we treat the dataset as 0–1 and scale.
fn parse_rows(body: &str) -> Vec<Row> {
    let Ok(v) = serde_json::from_str::<Value>(body) else {
        return Vec::new();
    };
    let entries = v
        .get("data")
        .and_then(Value::as_array)
        .or_else(|| v.as_array());
    let Some(entries) = entries else {
        return Vec::new();
    };
    let mut raw: Vec<Row> = Vec::new();
    for e in entries {
        let name = e
            .get("name")
            .or_else(|| e.get("slug"))
            .or_else(|| e.get("model_name"))
            .and_then(Value::as_str);
        let Some(name) = name else { continue };
        let intel = find_f64(e, "artificial_analysis_intelligence_index");
        let coding = find_f64(e, "artificial_analysis_coding_index");
        // A model AA has only partly evaluated is still a rated model: dropping it left a new
        // release scored by its predecessor's family entry. The missing coding index is
        // estimated later, in `rows_to_scores`, never copied from the intelligence index.
        let Some(intel) = intel else {
            tracing::debug!(
                model = name,
                "benchmark feed row has no intelligence index; skipped"
            );
            continue;
        };
        raw.push((name.to_string(), intel, coding));
    }
    let max = raw
        .iter()
        .flat_map(|(_, i, c)| std::iter::once(*i).chain(*c))
        .fold(0.0_f64, f64::max);
    let scale = if max <= 1.5 { 100.0 } else { 1.0 };
    for (_, i, c) in &mut raw {
        *i *= scale;
        if let Some(c) = c {
            *c *= scale;
        }
    }
    raw
}

/// How many rated rows nearest in intelligence inform one coding estimate.
const CODING_NEIGHBOURS: usize = 15;

/// Estimate a coding index AA has not published from the rows that carry both indices.
///
/// The two indices sit on different scales (a frontier model reads ~50 intelligence, ~77 coding),
/// so substituting the intelligence index ranked most of the feed about 20 points low on code-heavy
/// work — including brand-new frontier releases, which AA scores for intelligence first. The ratio
/// between the indices falls as intelligence rises, so the estimate uses the median ratio of the
/// rated rows closest in intelligence. On the 2026-09-27 feed that is off by 4.7 points on average
/// (leave-one-out), against 21.3 for the substitution. It never exceeds the best measured coding
/// index: above the measured range the ratio is an extrapolation, not evidence.
fn estimate_coding(intel: f64, rated: &[(f64, f64)]) -> f64 {
    if rated.is_empty() {
        return intel;
    }
    let mut nearest: Vec<&(f64, f64)> = rated.iter().collect();
    nearest.sort_by(|a, b| (a.0 - intel).abs().total_cmp(&(b.0 - intel).abs()));
    let mut ratios: Vec<f64> = nearest
        .iter()
        .take(CODING_NEIGHBOURS)
        .map(|(i, c)| c / i)
        .collect();
    ratios.sort_by(f64::total_cmp);
    let mid = ratios.len() / 2;
    let median = if ratios.len().is_multiple_of(2) {
        (ratios[mid - 1] + ratios[mid]) / 2.0
    } else {
        ratios[mid]
    };
    let ceiling = rated.iter().map(|(_, c)| *c).fold(f64::MIN, f64::max);
    (intel * median).min(ceiling)
}

/// Every row with a coding index: `(name, intelligence, coding, coding_was_estimated)`.
fn resolve_coding(rows: &[Row]) -> Vec<(String, f64, f64, bool)> {
    let rated: Vec<(f64, f64)> = rows
        .iter()
        .filter_map(|(_, i, c)| c.filter(|_| *i > 0.0).map(|c| (*i, c)))
        .collect();
    rows.iter()
        .map(|(name, intel, coding)| match coding {
            Some(c) => (name.clone(), *intel, *c, false),
            None => (name.clone(), *intel, estimate_coding(*intel, &rated), true),
        })
        .collect()
}

fn rows_to_scores(rows: &[Row]) -> BenchmarkScores {
    let mut b = BenchmarkScores::new();
    for (name, intel, coding, _) in resolve_coding(rows) {
        b.insert(&name, intel, coding);
    }
    b
}

async fn fetch_api(url: &str, key: &str) -> Option<Vec<Row>> {
    let resp = forge_provider::bundled_http_client()
        .get(url)
        .header("x-api-key", key)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        tracing::debug!("benchmark API returned {}", resp.status());
        return None;
    }
    let body = resp.text().await.ok()?;
    let rows = parse_rows(&body);
    (!rows.is_empty()).then_some(rows)
}

/// Where each source lives. Production values come from [`Endpoints::live`]; tests point them at
/// a local server and a temp dir.
struct Endpoints {
    cache: PathBuf,
    api_url: String,
    mirror_urls: Vec<String>,
    key: Option<String>,
    mirror_enabled: bool,
}

impl Endpoints {
    fn live(mirror_enabled: bool) -> Self {
        Self {
            cache: cache_path(),
            api_url: API_URL.to_string(),
            mirror_urls: vec![
                mirror::SITE_URL.to_string(),
                mirror::RELEASE_URL.to_string(),
            ],
            key: forge_config::benchmark_api_key(),
            mirror_enabled,
        }
    }
}

/// `FORGE_BENCHMARK_MIRROR=0` (or `false`/`off`/`no`) turns the mirror off without editing config.
fn mirror_env_allows() -> bool {
    !std::env::var("FORGE_BENCHMARK_MIRROR").is_ok_and(|v| {
        matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "off" | "no"
        )
    })
}

/// Catalog models still unmatched by `scores`, remembered so they don't re-trigger fetches.
fn unmatched(model_ids: &[String], scores: &BenchmarkScores) -> Vec<String> {
    model_ids
        .iter()
        .filter(|m| scores.source_score_for(m).is_none())
        .cloned()
        .collect()
}

/// Benchmark scores for ranking the given catalog `model_ids`. Incremental + cache-first: cached
/// scores are reused, and the network is touched only when something needs it (a new catalog
/// model, an expired TTL, `force`). Source precedence: the user's own key, then the keyless Forge
/// mirror, then the snapshot bundled into this build. `mesh.benchmark_ranking = false` disables
/// the whole layer; `mesh.benchmark_mirror = false` (or `FORGE_BENCHMARK_MIRROR=0`) keeps Forge
/// off the network for benchmarks unless the user supplied their own key.
pub async fn ensure(
    config: &forge_config::Config,
    model_ids: &[String],
    force: bool,
) -> Option<BenchmarkScores> {
    if !config.mesh.benchmark_ranking {
        return None;
    }
    let endpoints = Endpoints::live(config.mesh.benchmark_mirror && mirror_env_allows());
    resolve(&endpoints, model_ids, force).await
}

async fn resolve(ep: &Endpoints, model_ids: &[String], force: bool) -> Option<BenchmarkScores> {
    let cached = load_cache(&ep.cache);
    let cached_scores = cached.as_ref().map(|c| rows_to_scores(&c.rows));
    let has_new_model = |c: &Cache| {
        model_ids.iter().any(|m| {
            !c.unrated.contains(m)
                && cached_scores
                    .as_ref()
                    .is_none_or(|s| s.source_score_for(m).is_none())
        })
    };
    let may_retry = |c: &Cache| now() - c.checked_at >= RETRY_SECS;

    if let Some(key) = ep.key.as_deref() {
        // Own key: a cache from another source is upgraded once, then the rules are the original
        // ones (new catalog model, 30-day backstop, daily refresh while coding indices are missing).
        let needs = force
            || match &cached {
                None => true,
                Some(c) if c.source != Source::OwnKey => may_retry(c),
                Some(c) => {
                    c.age > SAFETY_TTL_SECS
                        || (c.age > PARTIAL_TTL_SECS && c.rows.iter().any(|(_, _, c)| c.is_none()))
                        || has_new_model(c)
                }
            };
        if needs {
            if let Some(rows) = fetch_api(&ep.api_url, key).await {
                let scores = rows_to_scores(&rows);
                save_cache(
                    &ep.cache,
                    &Entry {
                        rows: &rows,
                        unrated: &unmatched(model_ids, &scores),
                        source: Source::OwnKey,
                        etag: None,
                        data_as_of: None,
                    },
                );
                return Some(scores);
            }
            if cached.is_some() {
                touch(&ep.cache, None);
            }
        }
        if cached_scores.is_some() {
            return cached_scores;
        }
    }

    let needs_mirror = ep.mirror_enabled
        && (force
            || match &cached {
                None => true,
                Some(c) => may_retry(c) && (c.age > MIRROR_TTL_SECS || has_new_model(c)),
            });
    if needs_mirror {
        let etag = cached
            .as_ref()
            .filter(|c| c.source == Source::Mirror && !force)
            .and_then(|c| c.etag.as_ref());
        match mirror::fetch_mirror(&ep.mirror_urls, etag).await {
            Some(mirror::Fetched::Fresh { mirror, url, etag }) => {
                let scores = rows_to_scores(&mirror.rows);
                save_cache(
                    &ep.cache,
                    &Entry {
                        rows: &mirror.rows,
                        unrated: &unmatched(model_ids, &scores),
                        source: Source::Mirror,
                        etag: etag.as_deref().map(|t| (url.as_str(), t)),
                        data_as_of: Some(mirror.source_fetched_at),
                    },
                );
                return Some(scores);
            }
            Some(mirror::Fetched::NotModified) => {
                if let Some(scores) = &cached_scores {
                    touch(&ep.cache, Some(&unmatched(model_ids, scores)));
                    return cached_scores;
                }
            }
            None => {
                if cached.is_some() {
                    touch(&ep.cache, None);
                }
            }
        }
    }
    if cached_scores.is_some() {
        return cached_scores;
    }

    // Nothing cached and nothing reachable: seed the cache from the bundled snapshot so the next
    // run does not wait on the network again before the retry interval has passed.
    let snap = mirror::snapshot()?;
    let scores = rows_to_scores(&snap.rows);
    save_cache(
        &ep.cache,
        &Entry {
            rows: &snap.rows,
            unrated: &unmatched(model_ids, &scores),
            source: Source::Snapshot,
            etag: None,
            data_as_of: Some(snap.source_fetched_at),
        },
    );
    Some(scores)
}

fn fmt_age(secs: i64) -> String {
    match secs.max(0) {
        s if s < 90 => "moments".to_string(),
        s if s < 3600 * 2 => format!("{} min", s / 60),
        s if s < 3600 * 48 => format!("{} h", s / 3600),
        s => format!("{} days", s / 86400),
    }
}

/// One line for `forge benchmarks`: which source the scores in use came from and how old they are.
pub(crate) fn provenance_line() -> Option<String> {
    let c = load_cache(&cache_path())?;
    Some(format!(
        "source: {} · data from {} ago",
        c.source.label(),
        fmt_age(now() - c.data_as_of)
    ))
}

#[cfg(test)]
mod ensure_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_flat_and_nested_shapes() {
        let flat = r#"{"data":[{"name":"GPT-5.2","artificial_analysis_intelligence_index":58,"artificial_analysis_coding_index":55}]}"#;
        let r = parse_rows(flat);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0], ("GPT-5.2".into(), 58.0, Some(55.0)));

        let nested = r#"[{"slug":"claude-opus","evaluations":{"artificial_analysis_intelligence_index":0.64}}]"#;
        let r = parse_rows(nested);
        assert_eq!(r[0].0, "claude-opus");
        assert_eq!(r[0].1, 64.0, "all-fraction dataset normalised to 0–100");
        assert_eq!(r[0].2, None, "a missing coding index stays missing");
    }

    #[test]
    fn weak_model_in_a_0_100_dataset_is_not_inflated() {
        // A real 0–100 dataset: a strong model at 58 and a genuinely weak one scoring 1.3.
        // The old per-value rule turned 1.3 into 130; dataset-level scaling must leave it at 1.3.
        let body = r#"{"data":[
            {"name":"GPT-5.5","artificial_analysis_intelligence_index":58,"artificial_analysis_coding_index":55},
            {"name":"Tiny 0.6B","artificial_analysis_intelligence_index":1.3,"artificial_analysis_coding_index":1.4}
        ]}"#;
        let r = parse_rows(body);
        let tiny = r.iter().find(|(n, ..)| n == "Tiny 0.6B").unwrap();
        assert_eq!(tiny.1, 1.3, "weak score stays 1.3, not 130");
        assert_eq!(tiny.2, Some(1.4));
    }

    #[test]
    fn a_missing_coding_index_is_estimated_on_the_coding_scale() {
        // Real rows from the 2026-09-27 feed. GPT-6 Sol has no coding index yet; copying its
        // intelligence (47.5) ranked it 30 points under its predecessor on code-heavy work.
        let rows: Vec<Row> = vec![
            ("GPT-5.6 Sol (max)".into(), 47.0, Some(77.4)),
            ("GPT-6 Astra (max)".into(), 52.7, Some(76.9)),
            (
                "Claude Opus 5 (Adaptive Reasoning, Max Effort)".into(),
                50.8,
                Some(78.0),
            ),
            (
                "Claude Fable 5.1 (Adaptive Reasoning, Max Effort, Default Fallback)".into(),
                53.4,
                Some(81.6),
            ),
            ("GPT-6 Sol (max)".into(), 47.5, None),
            (
                "Claude Opus 5.5 (Adaptive Reasoning, Max Effort, Default Fallback)".into(),
                57.6,
                None,
            ),
        ];
        let b = rows_to_scores(&rows);
        let sol = b.score_for("codex-oauth::gpt-6-sol").unwrap();
        assert_eq!(sol.intelligence, 47.5);
        assert!(
            (70.0..=81.6).contains(&sol.coding),
            "estimated on the coding scale: {}",
            sol.coding
        );
        let opus = b.score_for("anthropic::claude-opus-5-5").unwrap();
        assert_eq!(
            opus.coding, 81.6,
            "capped at the best measured coding index"
        );
    }

    #[test]
    fn a_published_coding_index_is_never_replaced() {
        let rows: Vec<Row> = vec![
            ("GPT-5.6 Sol (max)".into(), 47.0, Some(77.4)),
            ("GPT-6 Sol (max)".into(), 47.5, None),
        ];
        let b = rows_to_scores(&rows);
        assert_eq!(
            b.score_for("codex-oauth::gpt-5.6-sol").unwrap().coding,
            77.4
        );
    }
}
