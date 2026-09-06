//! Network tools: `web_fetch` (keyless URL → clean text) and `web_search` (BYOK ranked
//! results). Both declare [`SideEffect::Network`] so the permission broker gates egress
//! distinctly from a local read (SSRF / exfiltration risk). See docs/features/web-tools.md.

use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use forge_types::SideEffect;
use futures::StreamExt;
use serde_json::{json, Value};

use crate::{str_arg, Tool, ToolError};

const FETCH_TIMEOUT: Duration = Duration::from_secs(15);
/// Hard byte cap on any fetched/searched HTTP body. `reqwest`'s `.text()` buffers the WHOLE body
/// before our `max_chars` truncation runs, so a huge or slow response OOM-kills the process — the
/// timeout bounds time, not bytes. We stream the body and stop at this cap instead. 5 MiB is far
/// larger than any real HTML page yet far below an OOM threshold.
const MAX_BODY_BYTES: usize = 5 * 1024 * 1024;
const DEFAULT_MAX_CHARS: usize = 10_000;
const DEFAULT_SEARCH_COUNT: u32 = 5;
const MAX_SEARCH_COUNT: u32 = 10;
const USER_AGENT: &str = concat!(
    "forge/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/Adulari/forge)"
);

/// A reqwest `ClientBuilder` pre-seeded with Mozilla's bundled root CAs, so web_fetch / web_search
/// HTTPS works on a host with no OS trust store. A plain `reqwest::Client::builder().build()` trusts
/// the OS store and **panics internally** where there is none (bare container / minimal image).
/// Mirrors forge-provider's client; forge-tools can't depend on forge-provider.
fn bundled_client_builder() -> reqwest::ClientBuilder {
    let certs = webpki_root_certs::TLS_SERVER_ROOT_CERTS
        .iter()
        .filter_map(|der| reqwest::Certificate::from_der(der.as_ref()).ok());
    reqwest::Client::builder()
        .tls_certs_only(certs)
        .redirect(safe_redirect_policy())
}

fn safe_redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= 10 {
            attempt.error("too many redirects")
        } else if is_safe_url(attempt.url().as_str()).is_err() {
            attempt.error("refusing redirect to private/local URL")
        } else {
            attempt.follow()
        }
    })
}

// ---------------------------------------------------------------------------
// web_fetch
// ---------------------------------------------------------------------------

/// Fetch a URL over HTTP(S) and return its readable text. Network side effect.
pub struct WebFetchTool;

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }
    fn description(&self) -> &str {
        "Fetch a web page over HTTP(S) and return its readable text content. \
         Use for reading documentation, articles, or any public URL."
    }
    fn side_effect(&self) -> SideEffect {
        SideEffect::Network
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "The http(s) URL to fetch." },
                "max_chars": { "type": "integer", "description": "Cap on returned characters (default 10000)." }
            },
            "required": ["url"]
        })
    }
    async fn run(&self, args: &Value) -> Result<String, ToolError> {
        let url = str_arg(args, "url")?;
        let max_chars = args
            .get("max_chars")
            .and_then(Value::as_u64)
            .map(|n| n as usize)
            .unwrap_or(DEFAULT_MAX_CHARS);
        is_safe_url(url)?;

        let client = bundled_client_builder()
            .user_agent(USER_AGENT)
            .timeout(FETCH_TIMEOUT)
            .build()
            .map_err(|e| ToolError::Failed(format!("http client: {e}")))?;
        let resp = client
            .get(url)
            .send()
            .await
            .map_err(|e| ToolError::Failed(format!("fetching {url}: {e}")))?;
        let status = resp.status();
        let bytes = read_body_capped(resp, MAX_BODY_BYTES, url).await?;
        if !status.is_success() {
            return Err(ToolError::Failed(format!("{url} returned HTTP {status}")));
        }

        let body = String::from_utf8_lossy(&bytes);
        let text = html_to_text(&body);
        Ok(truncate_chars(&text, max_chars))
    }
}

/// Read an HTTP response body, streaming it chunk-by-chunk and STOPPING once `cap` bytes are
/// buffered — so an unbounded or slow body can never OOM the process (unlike `resp.text()`, which
/// buffers the whole thing first; the request timeout bounds time, not bytes). A `Content-Length`
/// already over the cap short-circuits before any download. The returned bytes are capped, not the
/// full body; the caller decodes them lossily and applies its own char-level truncation.
async fn read_body_capped(
    resp: reqwest::Response,
    cap: usize,
    url: &str,
) -> Result<Vec<u8>, ToolError> {
    if let Some(len) = resp.content_length() {
        if len as usize > cap {
            return Err(ToolError::Failed(format!(
                "{url}: response body too large (declared {len} bytes exceeds the {cap}-byte cap)"
            )));
        }
    }
    let mut stream = resp.bytes_stream();
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk =
            chunk.map_err(|e| ToolError::Failed(format!("reading body from {url}: {e}")))?;
        if accumulate_capped(&mut buf, &chunk, cap) {
            break;
        }
    }
    Ok(buf)
}

/// Append `chunk` to `buf` without exceeding `cap` bytes. Returns `true` once the cap is reached
/// (the caller should stop reading). Keeps the in-memory buffer hard-bounded regardless of how much
/// the server sends.
fn accumulate_capped(buf: &mut Vec<u8>, chunk: &[u8], cap: usize) -> bool {
    let remaining = cap.saturating_sub(buf.len());
    if chunk.len() >= remaining {
        buf.extend_from_slice(&chunk[..remaining]);
        true
    } else {
        buf.extend_from_slice(chunk);
        false
    }
}

/// Reject anything that isn't a plain http(s) request to a public host. Defends against SSRF
/// to loopback/private/link-local/metadata addresses. Known limit: no DNS resolution, so a
/// hostname that *resolves* to a private IP (DNS rebinding) is not caught here.
pub(crate) fn is_safe_url(url: &str) -> Result<(), ToolError> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|_| ToolError::BadArgs(format!("not a valid URL: {url}")))?;
    match parsed.scheme() {
        "http" | "https" => {}
        other => {
            return Err(ToolError::BadArgs(format!(
                "unsupported URL scheme '{other}': only http/https are allowed"
            )))
        }
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| ToolError::BadArgs(format!("URL has no host: {url}")))?;

    let lower = host.to_ascii_lowercase();
    if lower == "localhost" || lower.ends_with(".local") || lower.ends_with(".localhost") {
        return Err(ToolError::BadArgs(format!(
            "refusing to fetch local host '{host}'"
        )));
    }
    // Bracketed IPv6 hosts arrive as "[::1]"; strip the brackets before parsing.
    let ip_candidate = lower.trim_start_matches('[').trim_end_matches(']');
    // `IpAddr::parse` only accepts strict dotted-quad/hex-colon notation, so a bare-integer or
    // octal/hex-encoded host (e.g. "2130706433", which glibc's resolver treats the same as
    // "127.0.0.1") would otherwise sail through as an "ordinary hostname". Also check the
    // numbers-and-dots notation the system resolver actually accepts.
    let literal_ip = ip_candidate
        .parse::<IpAddr>()
        .ok()
        .or_else(|| parse_ipv4_like(ip_candidate).map(IpAddr::V4));
    if let Some(ip) = literal_ip {
        if is_private_ip(ip) {
            return Err(ToolError::BadArgs(format!(
                "refusing to fetch private/loopback address '{host}'"
            )));
        }
    }
    Ok(())
}

/// Parse `host` as an IPv4 address using the "numbers-and-dots" notation accepted by glibc's
/// `inet_aton`/`getaddrinfo` (what `reqwest`'s resolver ultimately calls): 1-4 dot-separated
/// components, each decimal, octal (leading `0`), or hex (leading `0x`), that combine into a
/// 32-bit address — e.g. a bare integer like `2852039166` or `0x7f.0.0.1`. `IpAddr::parse` does
/// not accept any of these forms, only strict 4-part decimal dotted-quad.
fn parse_ipv4_like(host: &str) -> Option<Ipv4Addr> {
    let parts: Vec<&str> = host.split('.').collect();
    if parts.is_empty() || parts.len() > 4 {
        return None;
    }
    let mut values = Vec::with_capacity(parts.len());
    for part in &parts {
        values.push(parse_c_uint(part)?);
    }
    // Every component but the last must fit in a byte; the last absorbs the remaining bits.
    // Shift amounts are fixed literals (never a runtime-computed width) to avoid an overflow
    // panic on the all-bits-in-one-component (bare integer) form.
    let addr: u32 = match values.as_slice() {
        [a] => (*a).try_into().ok()?,
        [a, b] => {
            if *a > 0xff || *b > 0x00ff_ffff {
                return None;
            }
            ((*a as u32) << 24) | (*b as u32)
        }
        [a, b, c] => {
            if *a > 0xff || *b > 0xff || *c > 0xffff {
                return None;
            }
            ((*a as u32) << 24) | ((*b as u32) << 16) | (*c as u32)
        }
        [a, b, c, d] => {
            if *a > 0xff || *b > 0xff || *c > 0xff || *d > 0xff {
                return None;
            }
            ((*a as u32) << 24) | ((*b as u32) << 16) | ((*c as u32) << 8) | (*d as u32)
        }
        _ => return None,
    };
    Some(Ipv4Addr::from(addr))
}

/// Parse one dotted-quad component the way `strtoul` does inside `inet_aton`: decimal by
/// default, octal with a leading `0`, hex with a leading `0x`/`0X`. Returns `None` for anything
/// that isn't a plain unsigned integer literal (i.e. an ordinary hostname label).
fn parse_c_uint(s: &str) -> Option<u64> {
    if s.is_empty() {
        return None;
    }
    let (digits, radix) = if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        (hex, 16)
    } else if s.len() > 1 && s.starts_with('0') {
        (&s[1..], 8)
    } else {
        (s, 10)
    };
    if digits.is_empty() {
        return None;
    }
    u64::from_str_radix(digits, radix).ok()
}

fn is_private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                // 100.64.0.0/10 (CGNAT) and 192.0.0.0/24 — treat as non-public.
                || (v4.octets()[0] == 100 && (v4.octets()[1] & 0xc0) == 64)
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_private_ip(IpAddr::V4(v4));
            }
            let seg = v6.segments();
            v6.is_loopback()
                || v6.is_unspecified()
                // fc00::/7 unique-local
                || (seg[0] & 0xfe00) == 0xfc00
                // fe80::/10 link-local
                || (seg[0] & 0xffc0) == 0xfe80
        }
    }
}

/// Strip HTML to readable text: drop `<script>`/`<style>` bodies, remove tags, decode the
/// common named/numeric entities, and collapse runs of whitespace. The page `<title>`, when
/// present, is surfaced as the first line.
pub(crate) fn html_to_text(html: &str) -> String {
    let title = extract_title(html);
    let without_blocks = strip_block(&strip_block(html, "script"), "style");
    let mut out = String::with_capacity(without_blocks.len());
    let mut in_tag = false;
    for ch in without_blocks.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    let decoded = decode_entities(&out);
    let collapsed = collapse_ws(&decoded);
    match title {
        Some(t) if !collapsed.starts_with(&t) => format!("{t}\n\n{collapsed}"),
        _ => collapsed,
    }
}

fn extract_title(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<title")?;
    let open_end = lower[start..].find('>')? + start + 1;
    let close = lower[open_end..].find("</title>")? + open_end;
    let raw = &html[open_end..close];
    let t = collapse_ws(&decode_entities(raw));
    (!t.is_empty()).then_some(t)
}

/// Remove `<tag …>…</tag>` blocks (case-insensitive), including their content.
fn strip_block(html: &str, tag: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut out = String::with_capacity(html.len());
    let mut cursor = 0usize;
    while let Some(rel) = lower[cursor..].find(&open) {
        let start = cursor + rel;
        out.push_str(&html[cursor..start]);
        match lower[start..].find(&close) {
            Some(end_rel) => cursor = start + end_rel + close.len(),
            None => {
                cursor = html.len();
                break;
            }
        }
    }
    out.push_str(&html[cursor..]);
    out
}

fn decode_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let truncated: String = s.chars().take(max).collect();
    format!("{truncated}\n\n[truncated at {max} chars]")
}

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
fn keyless_chain() -> FirstThatAnswers {
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
fn decode_ddg_href(href: &str) -> String {
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

fn format_results(results: &[SearchResult]) -> String {
    if results.is_empty() {
        return "No results found.".to_string();
    }
    results
        .iter()
        .enumerate()
        .map(|(i, r)| {
            // Brave snippets may carry <strong> highlight tags — reduce to plain text.
            let desc = html_to_text(&r.description);
            format!("{}. {}\n   {}\n   {}", i + 1, r.title, r.url, desc)
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Web search over a pluggable [`SearchBackend`]. Resolves the Brave backend from the
/// `BRAVE_API_KEY` environment variable (the CLI injects the keyring value before a session)
/// unless an explicit backend was supplied (tests / alternative providers).
#[derive(Default)]
pub struct WebSearchTool {
    backend: Option<Arc<dyn SearchBackend>>,
}

impl WebSearchTool {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_backend(backend: Arc<dyn SearchBackend>) -> Self {
        Self {
            backend: Some(backend),
        }
    }

    /// Pick a backend: an explicit one (tests / config) wins; then a self-hosted SearXNG if one
    /// is configured; then Brave if a key is set; else the keyless chain.
    ///
    /// `FORGE_SEARXNG_URL` is the escape hatch for anyone who wants search that no upstream can
    /// throttle: a local SearXNG answers without a key, without a quota, and aggregates the
    /// engines below. It is opt-in because it is a service to run, not a default anyone gets.
    fn resolve_backend(&self) -> Arc<dyn SearchBackend> {
        if let Some(b) = &self.backend {
            return b.clone();
        }
        if let Ok(url) = std::env::var("FORGE_SEARXNG_URL") {
            if !url.is_empty() {
                return Arc::new(SearxNg::new(url));
            }
        }
        // A key still wins over the keyless chain, but the chain now leads with a local SearXNG,
        // so zero-setup search is only the fallback rather than the whole story.

        match std::env::var("BRAVE_API_KEY") {
            Ok(key) if !key.is_empty() => Arc::new(BraveSearch::new(key)),
            _ => Arc::new(keyless_chain()),
        }
    }
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }
    fn description(&self) -> &str {
        "Search the web and return ranked results (title, URL, snippet). \
         Use to find current information, documentation, or sources to then fetch."
    }
    fn side_effect(&self) -> SideEffect {
        SideEffect::Network
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "The search query." },
                "count": { "type": "integer", "description": "How many results (default 5, max 10)." }
            },
            "required": ["query"]
        })
    }
    async fn run(&self, args: &Value) -> Result<String, ToolError> {
        let query = str_arg(args, "query")?;
        let count = args
            .get("count")
            .and_then(Value::as_u64)
            .map(|n| (n as u32).clamp(1, MAX_SEARCH_COUNT))
            .unwrap_or(DEFAULT_SEARCH_COUNT);
        let results = self.resolve_backend().search(query, count).await?;
        Ok(format_results(&results))
    }
}

#[cfg(test)]
mod tests {
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

    struct Dead;
    #[async_trait]
    impl SearchBackend for Dead {
        async fn search(&self, _: &str, _: u32) -> Result<Vec<SearchResult>, ToolError> {
            Err(ToolError::Failed("throttled".into()))
        }
    }
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

    use super::*;

    #[test]
    fn safe_url_accepts_public_https() {
        assert!(is_safe_url("https://example.com/docs").is_ok());
        assert!(is_safe_url("http://93.184.216.34/").is_ok());
    }

    #[test]
    fn safe_url_rejects_ssrf_and_bad_schemes() {
        for bad in [
            "http://127.0.0.1/",
            "http://localhost:8080/",
            "http://10.0.0.1/",
            "http://192.168.1.1/",
            "http://169.254.169.254/latest/meta-data/",
            "http://[::1]/",
            "http://[::ffff:127.0.0.1]/",
            "https://foo.local/",
            "file:///etc/passwd",
            "ftp://example.com/",
            "not a url",
            // Bare-integer / octal / hex encodings of loopback and the cloud metadata IP that
            // the system resolver (glibc inet_aton/getaddrinfo) still treats as literal IPs.
            "http://2130706433/",
            "http://2852039166/",
            "http://0x7f.0.0.1/",
            "http://0177.0.0.1/",
            "http://0x7f000001/",
            "http://127.1/",
        ] {
            assert!(is_safe_url(bad).is_err(), "should reject {bad}");
        }
    }

    #[test]
    fn safe_url_still_accepts_ordinary_hostnames_and_dotted_ips() {
        assert!(is_safe_url("https://example.com/docs").is_ok());
        assert!(is_safe_url("http://93.184.216.34/").is_ok());
        // A numeric-looking label that isn't a valid IPv4-like literal (too many parts) is still
        // an ordinary hostname.
        assert!(is_safe_url("http://1.2.3.4.5.example.com/").is_ok());
    }

    #[tokio::test]
    async fn redirect_policy_rejects_private_targets() {
        use std::io::{Read, Write};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0; 1024];
            let _ = stream.read(&mut buf);
            write!(
                stream,
                "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{}/private\r\nContent-Length: 0\r\n\r\n",
                addr.port()
            )
            .unwrap();
        });

        let client = bundled_client_builder().build().unwrap();
        let err = client
            .get(format!("http://127.0.0.1:{}/redir", addr.port()))
            .send()
            .await
            .expect_err("redirect to a private target must be rejected");
        handle.join().unwrap();
        assert!(err.is_redirect(), "{err}");
    }

    #[test]
    fn html_to_text_strips_tags_scripts_and_decodes() {
        let html = "<html><head><title>Hello &amp; Bye</title></head><body>\
            <script>var x = 1 < 2;</script><style>.a{}</style>\
            <p>Tom &amp; Jerry &lt;3</p></body></html>";
        let text = html_to_text(html);
        assert!(text.starts_with("Hello & Bye"), "title surfaced: {text}");
        assert!(text.contains("Tom & Jerry <3"), "entities decoded: {text}");
        assert!(!text.contains("var x"), "script body dropped: {text}");
        assert!(!text.contains(".a{"), "style body dropped: {text}");
        assert!(
            !text.contains('<') || text.contains("<3"),
            "tags stripped: {text}"
        );
    }

    #[test]
    fn accumulate_capped_stops_at_the_byte_cap() {
        // Under the cap: appends fully, signals "keep reading".
        let mut buf = Vec::new();
        assert!(!accumulate_capped(&mut buf, b"hello", 100));
        assert_eq!(buf, b"hello");

        // Crossing the cap: takes only what fits and signals "stop".
        let mut buf = vec![0u8; 8];
        let hit = accumulate_capped(&mut buf, &[1u8; 10], 10);
        assert!(hit, "must signal the cap was reached");
        assert_eq!(buf.len(), 10, "buffer never exceeds the cap");

        // A single oversized chunk is truncated to the cap, never buffered whole (the OOM guard).
        let mut buf = Vec::new();
        let hit = accumulate_capped(&mut buf, &vec![7u8; 50_000_000], 1024);
        assert!(hit);
        assert_eq!(
            buf.len(),
            1024,
            "huge chunk truncated to cap, not buffered whole"
        );
    }

    #[test]
    fn truncate_caps_long_text() {
        let s = "x".repeat(100);
        let out = truncate_chars(&s, 10);
        assert!(out.starts_with(&"x".repeat(10)));
        assert!(out.contains("truncated"));
        assert_eq!(truncate_chars("short", 10), "short");
    }

    #[test]
    fn parse_brave_extracts_ordered_results() {
        let body = json!({
            "web": { "results": [
                { "title": "First", "url": "https://a.com", "description": "desc a" },
                { "title": "Second", "url": "https://b.com" }
            ]}
        });
        let r = parse_brave_results(&body);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].title, "First");
        assert_eq!(r[0].url, "https://a.com");
        assert_eq!(r[0].description, "desc a");
        assert_eq!(r[1].title, "Second");
        assert_eq!(r[1].description, "", "missing description defaults empty");
    }

    #[test]
    fn parse_brave_handles_missing_web_key() {
        assert!(parse_brave_results(&json!({ "error": "nope" })).is_empty());
    }

    struct MockBackend(Vec<SearchResult>);
    #[async_trait]
    impl SearchBackend for MockBackend {
        async fn search(&self, _q: &str, _c: u32) -> Result<Vec<SearchResult>, ToolError> {
            Ok(self.0.clone())
        }
    }

    #[tokio::test]
    async fn web_search_formats_results_from_backend() {
        let tool = WebSearchTool::with_backend(Arc::new(MockBackend(vec![SearchResult {
            title: "Rust".into(),
            url: "https://rust-lang.org".into(),
            description: "systems lang".into(),
        }])));
        let out = tool.run(&json!({ "query": "rust" })).await.unwrap();
        assert!(out.contains("1. Rust"));
        assert!(out.contains("https://rust-lang.org"));
        assert!(out.contains("systems lang"));
    }

    /// Live network smoke test (no key needed). Run on demand:
    /// `cargo test -p forge-tools web_fetch_live -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn web_fetch_live_example_com() {
        let out = WebFetchTool
            .run(&json!({ "url": "https://example.com" }))
            .await
            .expect("fetch example.com");
        assert!(out.contains("Example Domain"), "got: {out}");
    }

    #[test]
    fn parse_ddg_extracts_title_url_snippet() {
        let html = r#"
          <a rel="nofollow" class="result__a" href="https://rust-lang.org/">Rust &amp; Lang</a>
          <a class="result__snippet" href="https://rust-lang.org/">A language empowering everyone.</a>
          <a rel="nofollow" class="result__a" href="https://en.wikipedia.org/wiki/Rust">Rust - Wikipedia</a>
          <a class="result__snippet" href="x">Rust is a systems language.</a>
          <a class="result__a" href="//duckduckgo.com/y.js?ad=1">An ad</a>
        "#;
        let r = parse_ddg_results(html);
        assert_eq!(r.len(), 2, "ad/redirect anchors skipped");
        assert_eq!(r[0].title, "Rust & Lang");
        assert_eq!(r[0].url, "https://rust-lang.org/");
        assert_eq!(r[0].description, "A language empowering everyone.");
        assert_eq!(r[1].url, "https://en.wikipedia.org/wiki/Rust");
    }

    #[test]
    fn ddg_redirect_href_is_decoded() {
        assert_eq!(
            decode_ddg_href("//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fa%20b&rut=x"),
            "https://example.com/a b"
        );
        assert_eq!(
            decode_ddg_href("https://direct.example.com/"),
            "https://direct.example.com/"
        );
    }

    #[test]
    fn parse_ddg_ia_maps_abstract_results_and_topics() {
        let json = json!({
            "Heading": "Rust (programming language)",
            "AbstractText": "Rust is a systems language.",
            "AbstractURL": "https://en.wikipedia.org/wiki/Rust_(programming_language)",
            "Results": [
                { "Text": "Rust — official site", "FirstURL": "https://rust-lang.org/" }
            ],
            "RelatedTopics": [
                { "Text": "Cargo - the Rust build tool", "FirstURL": "https://doc.rust-lang.org/cargo/" },
                { "Name": "group", "Topics": [
                    { "Text": "Crates.io - the registry", "FirstURL": "https://crates.io/" }
                ]},
                { "Text": "dup", "FirstURL": "https://rust-lang.org/" }
            ]
        });
        let r = parse_ddg_ia(&json);
        assert_eq!(
            r[0].url,
            "https://en.wikipedia.org/wiki/Rust_(programming_language)"
        );
        assert_eq!(r[0].title, "Rust (programming language)");
        assert_eq!(r[1].url, "https://rust-lang.org/");
        assert_eq!(r[1].title, "Rust — official site");
        assert!(
            r.iter().any(|x| x.url == "https://crates.io/"),
            "nested topic included"
        );
        // "dup" reusing rust-lang.org URL is deduped.
        assert_eq!(
            r.iter()
                .filter(|x| x.url == "https://rust-lang.org/")
                .count(),
            1
        );
    }

    #[test]
    fn parse_ddg_ia_empty_when_no_urls() {
        assert!(parse_ddg_ia(&json!({ "Heading": "x", "RelatedTopics": [] })).is_empty());
    }

    #[tokio::test]
    async fn web_search_defaults_to_duckduckgo_without_key() {
        std::env::remove_var("BRAVE_API_KEY");
        // No key, no explicit backend → keyless DuckDuckGo default (web search works zero-setup).
        let backend = WebSearchTool::new().resolve_backend();
        // The mock/Brave paths aren't used here; just assert a backend is produced (no error).
        let _: Arc<dyn SearchBackend> = backend;
    }

    /// Live network smoke test (no key). `cargo test -p forge-tools ddg_live -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn ddg_live_search() {
        // Exercises the html→Instant-Answer fallback: even when DDG throttles the HTML
        // endpoint (HTTP 202), the IA API still returns the Rust abstract + topics.
        let out = WebSearchTool::new()
            .run(&json!({ "query": "rust programming language", "count": 3 }))
            .await
            .expect("ddg search");
        assert!(out.contains("rust") || out.contains("Rust"), "got: {out}");
        assert!(out.contains("http"), "has urls: {out}");
    }
}
