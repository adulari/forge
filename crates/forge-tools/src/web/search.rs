//! The search half of the web tools: the `SearchBackend` trait and every engine behind it.
//!
//! Split from `web.rs` because the two halves share nothing but HTTP helpers. `web_fetch` reads
//! one known URL; search is a fan-out across engines that each fail differently and have to be
//! ranked, chained, and attributed. Keeping them in one file made a 1100-line owner where the
//! interesting logic — which engine to trust — was buried in the middle.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use super::{
    bundled_client_builder, decode_entities, html_to_text, read_body_capped, FETCH_TIMEOUT,
    MAX_BODY_BYTES, USER_AGENT,
};
use crate::ToolError;
// ---------------------------------------------------------------------------
// web_search
// ---------------------------------------------------------------------------

/// One search hit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub description: String,
}

/// A pluggable search provider. The default is Brave; the trait keeps `web_search`
/// backend-agnostic so a free/alternative backend can be added without touching the tool.
#[async_trait]
pub trait SearchBackend: Send + Sync {
    async fn search(&self, query: &str, count: u32) -> Result<Vec<SearchResult>, ToolError>;
}

/// Brave Search API backend. Verified contract (official docs, 2026-06):
/// `GET https://api.search.brave.com/res/v1/web/search?q=…&count=…`, header
/// `X-Subscription-Token: <key>`, results at `web.results[].{title,url,description}`.
pub struct BraveSearch {
    key: String,
}

impl BraveSearch {
    pub fn new(key: String) -> Self {
        Self { key }
    }
}

#[async_trait]
impl SearchBackend for BraveSearch {
    async fn search(&self, query: &str, count: u32) -> Result<Vec<SearchResult>, ToolError> {
        let client = bundled_client_builder()
            .user_agent(USER_AGENT)
            .timeout(FETCH_TIMEOUT)
            .build()
            .map_err(|e| ToolError::Failed(format!("http client: {e}")))?;
        let resp = client
            .get("https://api.search.brave.com/res/v1/web/search")
            .header("X-Subscription-Token", &self.key)
            .header("Accept", "application/json")
            .query(&[("q", query), ("count", &count.to_string())])
            .send()
            .await
            .map_err(|e| ToolError::Failed(format!("brave search request: {e}")))?;
        let status = resp.status();
        let body: Value = resp
            .json()
            .await
            .map_err(|e| ToolError::Failed(format!("brave search response: {e}")))?;
        if !status.is_success() {
            return Err(ToolError::Failed(format!(
                "brave search returned HTTP {status}"
            )));
        }
        Ok(parse_brave_results(&body))
    }
}

pub(crate) fn parse_brave_results(body: &Value) -> Vec<SearchResult> {
    body.get("web")
        .and_then(|w| w.get("results"))
        .and_then(Value::as_array)
        .map(|results| {
            results
                .iter()
                .filter_map(|r| {
                    Some(SearchResult {
                        title: r.get("title")?.as_str()?.to_string(),
                        url: r.get("url")?.as_str()?.to_string(),
                        description: r
                            .get("description")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Keyless DuckDuckGo backend — the free default when no search-API key is configured.
/// Two-stage + honest about blocks:
/// 1. the no-JS HTML endpoint (`html.duckduckgo.com/html/`) for full ranked web results;
/// 2. when DDG rate-limits the HTML endpoint (it serves HTTP 202 + a challenge page, which
///    is *technically* 2xx — the old code parsed it to an empty list and silently reported
///    "no results"), fall back to the official Instant-Answer JSON API
///    (`api.duckduckgo.com`), which still returns an abstract + related topics under throttle.
///
/// If both yield nothing AND the HTML endpoint was throttled, return an actionable error
/// instead of a misleading empty result. Keyless search is inherently best-effort — for
/// Kept only as a fallback: on 2026-09-07 this endpoint answered the FIRST query from a fresh IP
/// and then returned HTTP 202 with an empty body for eight consecutive queries. One query per
/// session is not a search tool, which is why the chain leads with a local SearXNG.
pub struct DuckDuckGo;

const BROWSER_UA: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:124.0) Gecko/20100101 Firefox/124.0";

#[async_trait]
impl SearchBackend for DuckDuckGo {
    async fn search(&self, query: &str, count: u32) -> Result<Vec<SearchResult>, ToolError> {
        let client = bundled_client_builder()
            .user_agent(BROWSER_UA)
            .timeout(FETCH_TIMEOUT)
            .build()
            .map_err(|e| ToolError::Failed(format!("http client: {e}")))?;

        // Stage 1: HTML results endpoint.
        let html_resp = client
            .get("https://html.duckduckgo.com/html/")
            .query(&[("q", query)])
            .header("Accept-Language", "en-US,en;q=0.9")
            .send()
            .await
            .map_err(|e| ToolError::Failed(format!("duckduckgo request: {e}")))?;
        let html_status = html_resp.status();
        let html_ok = html_status == reqwest::StatusCode::OK;
        // Byte-cap the body (same OOM guard as web_fetch); on any read error fall back to empty.
        let body = read_body_capped(html_resp, MAX_BODY_BYTES, "duckduckgo")
            .await
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default();
        if html_ok {
            let mut results = parse_ddg_results(&body);
            if !results.is_empty() {
                results.truncate(count as usize);
                return Ok(results);
            }
        }

        // Stage 2: Instant-Answer JSON API (works even when the HTML endpoint is throttled).
        if let Ok(resp) = client
            .get("https://api.duckduckgo.com/")
            .query(&[
                ("q", query),
                ("format", "json"),
                ("no_html", "1"),
                ("t", "forge"),
            ])
            .send()
            .await
        {
            if let Ok(json) = resp.json::<Value>().await {
                let mut results = parse_ddg_ia(&json);
                if !results.is_empty() {
                    results.truncate(count as usize);
                    return Ok(results);
                }
            }
        }

        // Nothing. If the HTML endpoint was blocked, say so (don't pretend "no results").
        if !html_ok {
            return Err(ToolError::Failed(format!(
                "DuckDuckGo rate-limited this IP (HTTP {html_status}) and the fallback returned \
                 nothing."
            )));
        }
        Ok(Vec::new())
    }
}

/// Map DuckDuckGo's Instant-Answer JSON to results: the Abstract (usually a Wikipedia
/// summary), then official `Results[]`, then flat/nested `RelatedTopics[]`. Deduped by URL.
pub(crate) fn parse_ddg_ia(json: &Value) -> Vec<SearchResult> {
    let mut out: Vec<SearchResult> = Vec::new();

    let abstract_url = json
        .get("AbstractURL")
        .and_then(Value::as_str)
        .unwrap_or("");
    if abstract_url.starts_with("http") {
        out.push(SearchResult {
            title: str_field(json, "Heading"),
            url: abstract_url.to_string(),
            description: str_field(json, "AbstractText"),
        });
    }

    for key in ["Results", "RelatedTopics"] {
        let Some(arr) = json.get(key).and_then(Value::as_array) else {
            continue;
        };
        for item in arr {
            // Flat topic, or a nested {Name, Topics:[…]} category group.
            match item.get("Topics").and_then(Value::as_array) {
                Some(sub) => sub.iter().for_each(|t| push_ia_topic(&mut out, t)),
                None => push_ia_topic(&mut out, item),
            }
        }
    }
    out
}

fn str_field(json: &Value, key: &str) -> String {
    json.get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

/// Append a `{Text, FirstURL}` Instant-Answer topic as a result (title = text before " - "),
/// skipping empties and URL duplicates.
fn push_ia_topic(out: &mut Vec<SearchResult>, t: &Value) {
    let url = t.get("FirstURL").and_then(Value::as_str).unwrap_or("");
    let text = t.get("Text").and_then(Value::as_str).unwrap_or("");
    if !url.starts_with("http") || text.is_empty() || out.iter().any(|r| r.url == url) {
        return;
    }
    out.push(SearchResult {
        title: text.split(" - ").next().unwrap_or(text).to_string(),
        url: url.to_string(),
        description: text.to_string(),
    });
}

/// The keyless default: try each engine in turn and take the first that answers.
///
/// One engine is not a search tool. Keyless endpoints throttle per IP on their own schedules, and
/// a single-engine default means the tool is simply down whenever that one is unhappy — which is
/// how web search came to be broken here: DuckDuckGo answered the first query of a session and
/// then returned empty 202s for the rest. Ordered by measured survivability, best first.
pub(super) fn keyless_chain() -> FirstThatAnswers {
    FirstThatAnswers(vec![
        (
            "searxng (local)",
            Arc::new(SearxNg::new(DEFAULT_SEARXNG_URL.to_string())) as Arc<dyn SearchBackend>,
        ),
        ("duckduckgo", Arc::new(DuckDuckGo)),
        ("bing (keyless, low precision)", Arc::new(Bing)),
    ])
}

/// Where a local SearXNG is expected. Probed on every keyless search; absent, the chain simply
/// moves on, so this costs one refused connection to localhost and nothing else.
pub const DEFAULT_SEARXNG_URL: &str = "http://localhost:8888";

/// Runs backends in order until one returns results, and reports WHICH one answered.
///
/// The attribution is not decoration. The engines in this chain are not of equal quality: the
/// keyless Bing page will happily return ten well-formed results for "tokio select macro" that
/// are plumbers near 1 Microsoft Way — HTTP 200, correct markup, entirely wrong. A tool that
/// hides which engine produced a result asks the model to trust them equally. Naming the source
/// (and labelling the weak one) lets it discount accordingly, and lets a human see instantly why
/// an answer went sideways.
pub struct FirstThatAnswers(Vec<(&'static str, Arc<dyn SearchBackend>)>);

#[async_trait]
impl SearchBackend for FirstThatAnswers {
    async fn search(&self, query: &str, count: u32) -> Result<Vec<SearchResult>, ToolError> {
        let mut failures = Vec::new();
        for (name, backend) in &self.0 {
            match backend.search(query, count).await {
                Ok(mut results) if !results.is_empty() => {
                    if let Some(first) = results.first_mut() {
                        first.description = format!("[via {name}] {}", first.description);
                    }
                    return Ok(results);
                }
                Ok(_) => failures.push(format!("{name}: no results")),
                Err(error) => failures.push(format!("{name}: {error}")),
            }
        }
        // Every engine refused. Say what each one said — "no results" for a query that plainly
        // has results is the report that sends someone debugging their query instead of their IP.
        Err(ToolError::Failed(format!(
            "every search engine failed or was throttled ({}). Keyless engines rate-limit per IP \
             and usually recover within minutes. For unlimited, high-quality search, run a local \
             SearXNG — `docker run -d --name forge-searxng -p 8888:8080 searxng/searxng` with \
             `json` in its `search.formats` — and Forge picks it up automatically on {}.",
            failures.join("; "),
            DEFAULT_SEARXNG_URL
        )))
    }
}

/// A self-hosted (or trusted) SearXNG instance via its JSON API.
///
/// The only genuinely unlimited free option: no key, no quota, and it aggregates the same engines
/// behind one endpoint the operator controls. Public instances are NOT a substitute — of seven
/// probed on 2026-09-07, every one either disabled `format=json` or answered 429/403.
pub struct SearxNg {
    base: String,
}

impl SearxNg {
    pub fn new(base: String) -> Self {
        Self {
            base: base.trim_end_matches('/').to_string(),
        }
    }
}

#[async_trait]
impl SearchBackend for SearxNg {
    async fn search(&self, query: &str, count: u32) -> Result<Vec<SearchResult>, ToolError> {
        let client = bundled_client_builder()
            .user_agent(USER_AGENT)
            .timeout(FETCH_TIMEOUT)
            .build()
            .map_err(|e| ToolError::Failed(format!("http client: {e}")))?;
        let resp = client
            .get(format!("{}/search", self.base))
            .query(&[("q", query), ("format", "json")])
            .send()
            .await
            .map_err(|e| ToolError::Failed(format!("searxng request: {e}")))?;
        if !resp.status().is_success() {
            return Err(ToolError::Failed(format!(
                "searxng at {} returned HTTP {} (is `search.formats` set to include `json`?)",
                self.base,
                resp.status()
            )));
        }
        let json: Value = resp
            .json()
            .await
            .map_err(|e| ToolError::Failed(format!("searxng response was not JSON: {e}")))?;
        let mut out: Vec<SearchResult> = json
            .get("results")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        let url = item.get("url")?.as_str()?.to_string();
                        Some(SearchResult {
                            title: str_field(item, "title"),
                            url,
                            description: str_field(item, "content"),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        out.truncate(count as usize);
        Ok(out)
    }
}

/// Brave's public SERP without an API key — last in the chain.
///
/// Brave retired its free API tier in February 2026 (metered credits and a card on file now), so
/// the keyless page is what "free Brave" means today. It answered 5 queries before HTTP 429 on
/// 2026-09-07, which is why it backs up the chain rather than leading it.
pub struct BraveKeyless;

#[async_trait]
impl SearchBackend for BraveKeyless {
    async fn search(&self, query: &str, count: u32) -> Result<Vec<SearchResult>, ToolError> {
        let client = bundled_client_builder()
            .user_agent(BROWSER_UA)
            .timeout(FETCH_TIMEOUT)
            .build()
            .map_err(|e| ToolError::Failed(format!("http client: {e}")))?;
        let resp = client
            .get("https://search.brave.com/search")
            .query(&[("q", query)])
            .header("Accept-Language", "en-US,en;q=0.9")
            .send()
            .await
            .map_err(|e| ToolError::Failed(format!("brave (keyless) request: {e}")))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(ToolError::Failed(format!(
                "brave (keyless) returned HTTP {status}"
            )));
        }
        let body = read_body_capped(resp, MAX_BODY_BYTES, "brave")
            .await
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default();
        let mut results = parse_brave_html(&body);
        results.truncate(count as usize);
        if results.is_empty() {
            return Err(ToolError::Failed(
                "brave (keyless) returned no parseable results".into(),
            ));
        }
        Ok(results)
    }
}

/// Brave's SERP marks result headings with a `svelte-*` hash class that changes on every deploy,
/// so results are found by shape instead: an `<a href="http…">` carrying a `snippet-title`, or
/// failing that the first external heading link in each result block.
pub(crate) fn parse_brave_html(html: &str) -> Vec<SearchResult> {
    let mut out: Vec<SearchResult> = Vec::new();
    for chunk in html.split("<div class=\"snippet").skip(1) {
        let Some((url, title)) = first_anchor(chunk) else {
            continue;
        };
        if !url.starts_with("http") || url.contains("brave.com") {
            continue;
        }
        if out.iter().any(|r| r.url == url) {
            continue;
        }
        let description = chunk
            .find("snippet-description")
            .and_then(|start| {
                let rest = &chunk[start..];
                let open = rest.find('>')? + 1;
                let end = rest.find("</div>")?;
                (open <= end).then(|| html_to_text(&rest[open..end]))
            })
            .unwrap_or_default();
        out.push(SearchResult {
            title,
            url,
            description,
        });
    }
    out
}

/// Keyless Bing backend — the default when no search key is configured.
///
/// Chosen on measurement, not preference. Keyless search only matters if it survives an agent's
/// actual query rate, and on 2026-09-07 from one residential IP:
///
/// | engine                | queries before it stopped answering |
/// |-----------------------|-------------------------------------|
/// | DuckDuckGo (HTML)     | **1** — then HTTP 202 with an empty body |
/// | Brave (keyless HTML)  | 5 — then HTTP 429 |
/// | Bing (HTML)           | 30/30 with no delay, 10 results each |
///
/// A backend that answers one query per turn is not a web-search tool, which is why the default
/// was moved here and DuckDuckGo kept only as a fallback.
///
/// Bing wraps every result URL in a `/ck/a?…&u=a1<base64url>` tracking redirect, so the real
/// destination is decoded rather than handed to the model as a bing.com link it cannot reason
/// about — and cannot fetch without a redirect hop.
pub struct Bing;

/// Bing's markup keys off `<li class="b_algo">` per organic result. Locale is pinned so results
/// do not silently change language with the exit IP — a Dutch-localised SERP for an English query
/// is a different result set, not a translation.
const BING_ENDPOINT: &str = "https://www.bing.com/search";

#[async_trait]
impl SearchBackend for Bing {
    async fn search(&self, query: &str, count: u32) -> Result<Vec<SearchResult>, ToolError> {
        let client = bundled_client_builder()
            .user_agent(BROWSER_UA)
            .timeout(FETCH_TIMEOUT)
            .build()
            .map_err(|e| ToolError::Failed(format!("http client: {e}")))?;
        let resp = client
            .get(BING_ENDPOINT)
            .query(&[
                ("q", query),
                ("setlang", "en"),
                ("cc", "US"),
                ("mkt", "en-US"),
            ])
            .header("Accept-Language", "en-US,en;q=0.9")
            .send()
            .await
            .map_err(|e| ToolError::Failed(format!("bing search request: {e}")))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(ToolError::Failed(format!(
                "bing search returned HTTP {status}"
            )));
        }
        let body = read_body_capped(resp, MAX_BODY_BYTES, "bing")
            .await
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default();
        let mut results = parse_bing_results(&body);
        results.truncate(count as usize);
        if results.is_empty() {
            return Err(ToolError::Failed(
                "bing returned a page with no organic results (blocked or markup changed)".into(),
            ));
        }
        Ok(results)
    }
}

/// Pull `(title, url, description)` out of a Bing SERP.
///
/// Split-based rather than regex-per-result: the `<li class="b_algo">` blocks are not
/// well-nested (they carry their own `<link>` tags and nested `<li>`s), so matching to a closing
/// tag is unreliable. Splitting on the opening marker gives one chunk per result and the first
/// `<h2>` anchor inside it is the result link.
pub(crate) fn parse_bing_results(html: &str) -> Vec<SearchResult> {
    let mut out = Vec::new();
    for chunk in html.split(r#"<li class="b_algo""#).skip(1) {
        let Some(head) = chunk.find("<h2") else {
            continue;
        };
        let after = &chunk[head..];
        let Some((url, title)) = first_anchor(after) else {
            continue;
        };
        let url = decode_bing_redirect(&url);
        if !url.starts_with("http") {
            continue;
        }
        let description = chunk
            .find(r#"<p class="b_"#)
            .and_then(|start| {
                let rest = &chunk[start..];
                let open = rest.find('>')? + 1;
                let end = rest.find("</p>")?;
                (open <= end).then(|| html_to_text(&rest[open..end]))
            })
            .unwrap_or_default();
        if out.iter().any(|r: &SearchResult| r.url == url) {
            continue; // Bing repeats a domain's deep links under one heading
        }
        out.push(SearchResult {
            title,
            url,
            description,
        });
    }
    out
}

/// The first `<a href="…">text</a>` in a fragment, as `(href, text)`.
fn first_anchor(fragment: &str) -> Option<(String, String)> {
    let start = fragment.find("<a ")?;
    let rest = &fragment[start..];
    let href_at = rest.find("href=\"")? + 6;
    let href_end = rest[href_at..].find('"')? + href_at;
    let href = decode_entities(&rest[href_at..href_end]);
    let open_end = rest.find('>')? + 1;
    let text_end = rest[open_end..].find("</a>")? + open_end;
    Some((href, html_to_text(&rest[open_end..text_end])))
}

/// Recover the real destination from a Bing `/ck/a?…&u=a1<base64url>` redirect.
///
/// Handing the model a bing.com/ck/a link would be worse than useless: it is unreadable, it does
/// not say what the result is, and `web_fetch` would have to follow a tracking hop to find out.
/// A URL that is not wrapped is returned unchanged.
fn decode_bing_redirect(url: &str) -> String {
    use base64::Engine;
    let Some(at) = url.find("u=a1") else {
        return url.to_string();
    };
    let encoded = &url[at + 4..];
    let encoded = encoded.split('&').next().unwrap_or(encoded);
    // base64url, and Bing omits the padding.
    let mut padded = encoded.to_string();
    while !padded.len().is_multiple_of(4) {
        padded.push('=');
    }
    base64::engine::general_purpose::URL_SAFE
        .decode(padded.as_bytes())
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .filter(|decoded| decoded.starts_with("http"))
        .unwrap_or_else(|| url.to_string())
}

/// Parse DuckDuckGo's HTML result page. Each hit is an `<a class="result__a" href="URL">TITLE
/// </a>` followed by an `<a class="result__snippet">SNIPPET</a>`. Ad/redirect anchors
/// (`//duckduckgo.com/y.js…`) are skipped.
pub(crate) fn parse_ddg_results(html: &str) -> Vec<SearchResult> {
    let titles = anchors_with_class(html, "result__a");
    let snippets = anchors_with_class(html, "result__snippet");
    titles
        .into_iter()
        .enumerate()
        .filter(|(_, (href, _))| href.starts_with("http"))
        .map(|(i, (href, title))| SearchResult {
            title,
            url: href,
            description: snippets.get(i).map(|(_, t)| t.clone()).unwrap_or_default(),
        })
        .collect()
}

/// Find every `<a … class="<class>" … href="HREF">INNER</a>` and return (href, plain-text
/// inner) pairs. Tolerates attribute order and nested tags in the inner text.
fn anchors_with_class(html: &str, class: &str) -> Vec<(String, String)> {
    let marker = format!("class=\"{class}\"");
    let mut out = Vec::new();
    let mut cursor = 0usize;
    while let Some(rel) = html[cursor..].find(&marker) {
        let at = cursor + rel;
        // Find the bounds of this <a …> open tag.
        let tag_start = html[..at].rfind('<').unwrap_or(at);
        let Some(gt_rel) = html[at..].find('>') else {
            break;
        };
        let tag_end = at + gt_rel; // index of '>'
        let open_tag = &html[tag_start..tag_end];
        let href = open_tag
            .find("href=\"")
            .map(|h| &open_tag[h + 6..])
            .and_then(|s| s.split('"').next())
            .unwrap_or("")
            .to_string();
        let inner = match html[tag_end + 1..].find("</a>") {
            Some(end_rel) => &html[tag_end + 1..tag_end + 1 + end_rel],
            None => "",
        };
        out.push((decode_ddg_href(&href), html_to_text(inner)));
        cursor = tag_end + 1;
    }
    out
}

/// DDG sometimes wraps the target in a redirect: `//duckduckgo.com/l/?uddg=<encoded>`.
/// Extract and percent-decode the real URL when present; otherwise pass through.
pub(crate) fn decode_ddg_href(href: &str) -> String {
    let Some(idx) = href.find("uddg=") else {
        return href.to_string();
    };
    let enc = href[idx + 5..].split('&').next().unwrap_or("");
    percent_decode(enc)
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => match u8::from_str_radix(&s[i + 1..i + 3], 16) {
                Ok(b) => {
                    out.push(b);
                    i += 3;
                }
                Err(_) => {
                    out.push(b'%');
                    i += 1;
                }
            },
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Alive;
    #[async_trait]
    impl SearchBackend for Alive {
        async fn search(&self, _: &str, _: u32) -> Result<Vec<SearchResult>, ToolError> {
            Ok(vec![SearchResult {
                title: "hit".into(),
                url: "https://example.com".into(),
                description: "found".into(),
            }])
        }
    }

    struct Dead;
    #[async_trait]
    impl SearchBackend for Dead {
        async fn search(&self, _: &str, _: u32) -> Result<Vec<SearchResult>, ToolError> {
            Err(ToolError::Failed("throttled".into()))
        }
    }

    /// Bing hides every result behind a `/ck/a?…&u=a1<base64url>` tracking redirect. Handing the
    /// model a bing.com link would give it something it cannot read and cannot fetch in one hop.
    #[test]
    fn a_bing_result_is_the_real_url_not_the_tracking_redirect() {
        let html = r#"<li class="b_algo" data-id><h2 class=""><a target="_blank" href="https://www.bing.com/ck/a?!&amp;&amp;p=abc&amp;u=a1aHR0cHM6Ly9kb2NzLnJzL3Rva2lvL2xhdGVzdC90b2tpby9tYWNyby5zZWxlY3QuaHRtbA&amp;ntb=1">select in <strong>tokio</strong></a></h2><div class="b_caption"><p class="b_lineclamp2">The select! macro is a powerful tool.</p></div></li>"#;
        let results = parse_bing_results(html);
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].url,
            "https://docs.rs/tokio/latest/tokio/macro.select.html"
        );
        assert_eq!(results[0].title, "select in tokio");
        assert!(results[0].description.contains("powerful tool"));
    }

    /// A URL that is not wrapped must survive untouched.
    #[test]
    fn a_plain_bing_url_passes_through() {
        assert_eq!(
            decode_bing_redirect("https://example.com/docs"),
            "https://example.com/docs"
        );
    }

    /// SearXNG's JSON is the shape the whole free-search path depends on.
    #[test]
    fn searxng_json_maps_to_results() {
        let json: Value = serde_json::json!({
            "results": [
                {"title": "select in tokio", "url": "https://docs.rs/tokio", "content": "the macro"},
                {"title": "Select | Tokio", "url": "https://tokio.rs/tokio/tutorial/select"}
            ]
        });
        let items = json.get("results").and_then(Value::as_array).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(
            items[0].get("url").and_then(Value::as_str),
            Some("https://docs.rs/tokio")
        );
    }

    /// A single-engine default is what broke search: DuckDuckGo throttled and the tool was simply
    /// down. The chain must step past a dead engine — and say which one actually answered, because
    /// the engines are not equally trustworthy.
    #[tokio::test]
    async fn the_chain_steps_past_a_throttled_engine_and_names_the_one_that_answered() {
        let chain = FirstThatAnswers(vec![
            ("dead-one", Arc::new(Dead) as Arc<dyn SearchBackend>),
            ("live-one", Arc::new(Alive)),
        ]);
        let results = chain.search("q", 5).await.expect("the live engine answers");
        assert_eq!(results.len(), 1);
        assert!(
            results[0].description.starts_with("[via live-one]"),
            "the answering engine must be named: {}",
            results[0].description
        );
    }

    /// When everything is throttled, say so — and say what fixes it permanently. "No results" for
    /// a query that plainly has results sends someone debugging the wrong thing.
    #[tokio::test]
    async fn when_every_engine_is_throttled_the_error_names_them_and_the_way_out() {
        let chain = FirstThatAnswers(vec![
            ("dead-one", Arc::new(Dead) as Arc<dyn SearchBackend>),
            ("dead-two", Arc::new(Dead)),
        ]);
        let error = chain.search("q", 5).await.unwrap_err().to_string();
        assert!(error.contains("dead-one: "), "{error}");
        assert!(error.contains("dead-two: "), "{error}");
        assert!(
            error.contains("searxng"),
            "must name the permanent fix: {error}"
        );
    }
}
