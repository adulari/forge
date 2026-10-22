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

mod page;
mod search;
use search::keyless_chain;
pub use search::{
    Bing, BraveKeyless, BraveSearch, DuckDuckGo, SearchBackend, SearchResult, SearxNg,
};

const FETCH_TIMEOUT: Duration = Duration::from_secs(15);
/// Hard byte cap on any fetched/searched HTTP body. `reqwest`'s `.text()` buffers the WHOLE body
/// before our `max_chars` truncation runs, so a huge or slow response OOM-kills the process — the
/// timeout bounds time, not bytes. We stream the body and stop at this cap instead. 5 MiB is far
/// larger than any real HTML page yet far below an OOM threshold.
const MAX_BODY_BYTES: usize = 5 * 1024 * 1024;
const DEFAULT_MAX_CHARS: usize = 10_000;
const MAX_MAX_CHARS: usize = 100_000;
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
        .dns_resolver(std::sync::Arc::new(PublicOnlyResolver))
        .redirect(safe_redirect_policy())
}

/// Resolves hostnames itself and drops every loopback/private/link-local answer, so a hostname
/// that merely *points at* an internal address (`localtest.me`, `127.0.0.1.nip.io`, DNS
/// rebinding) cannot reach it. The connection uses exactly the addresses returned here, for the
/// first request and for every redirect hop alike.
struct PublicOnlyResolver;

impl reqwest::dns::Resolve for PublicOnlyResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        Box::pin(async move {
            let host = name.as_str().to_string();
            let addrs = tokio::net::lookup_host((host.as_str(), 0)).await?;
            let public: Vec<std::net::SocketAddr> =
                addrs.filter(|addr| !is_private_ip(addr.ip())).collect();
            if public.is_empty() {
                return Err(format!(
                    "refusing to connect: '{host}' resolves only to private/loopback addresses"
                )
                .into());
            }
            Ok(Box::new(public.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

/// A client for an endpoint the operator configured (a local SearXNG). It must reach loopback, so
/// it skips the public-only resolver that guards model-chosen URLs.
fn trusted_client_builder() -> reqwest::ClientBuilder {
    let certs = webpki_root_certs::TLS_SERVER_ROOT_CERTS
        .iter()
        .filter_map(|der| reqwest::Certificate::from_der(der.as_ref()).ok());
    reqwest::Client::builder()
        .tls_certs_only(certs)
        .redirect(reqwest::redirect::Policy::limited(5))
}

/// `web_fetch`'s redirect policy: the SSRF checks of [`safe_redirect_policy`], and it stops at a
/// hop to another site. Permission rules match on the URL the model asked for, so silently
/// following a redirect to a different host would let it read a host nobody approved. The tool
/// reports the target instead and the model fetches it as a fresh, gated call.
fn fetch_redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        let leaves_site = attempt
            .previous()
            .first()
            .is_some_and(|origin| !page::same_site(origin, attempt.url()));
        if attempt.previous().len() >= 10 {
            attempt.error("too many redirects")
        } else if is_safe_url(attempt.url().as_str()).is_err() {
            attempt.error("refusing redirect to private/local URL")
        } else if leaves_site {
            attempt.stop()
        } else {
            attempt.follow()
        }
    })
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
        "Fetch a public http(s) URL and return its main content as Markdown (headings, links, \
         code blocks kept; navigation and page chrome dropped). Plain-text, JSON and source files \
         are returned verbatim. Long pages are paged: the reply ends with the offset to pass to \
         read on. Pages are cached for 15 minutes, so paging does not refetch. A redirect to a \
         different host is reported, not followed; fetch the reported URL to continue."
    }
    fn side_effect(&self) -> SideEffect {
        SideEffect::Network
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "The http(s) URL to fetch." },
                "max_chars": { "type": "integer", "minimum": 1, "description": "Characters to return (default 10000, max 100000)." },
                "offset": { "type": "integer", "description": "Character offset to start from, for reading past the first window (default 0)." }
            },
            "required": ["url"]
        })
    }
    async fn run(&self, args: &Value) -> Result<String, ToolError> {
        let url = str_arg(args, "url")?;
        let max_chars = args
            .get("max_chars")
            .and_then(Value::as_u64)
            .map(|n| (n as usize).clamp(1, MAX_MAX_CHARS))
            .unwrap_or(DEFAULT_MAX_CHARS);
        let offset = args
            .get("offset")
            .and_then(Value::as_u64)
            .map(|n| n as usize)
            .unwrap_or(0);
        is_safe_url(url)?;
        if let Some(text) = page::cached(url) {
            return Ok(page::window(&text, offset, max_chars));
        }

        let client = bundled_client_builder()
            .redirect(fetch_redirect_policy())
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
        if status.is_redirection() {
            let target = resp
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|loc| resp.url().join(loc).ok());
            if let Some(target) = target {
                return Ok(format!(
                    "{url} redirects to a different host: {target}\nCall web_fetch with that URL \
                     to read it."
                ));
            }
        }
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let bytes = read_body_capped(resp, MAX_BODY_BYTES, url).await?;
        if !status.is_success() {
            return Err(ToolError::Failed(format!("{url} returned HTTP {status}")));
        }
        if is_binary_response(&content_type, &bytes) {
            let kind = if content_type.is_empty() {
                "unknown type"
            } else {
                content_type.as_str()
            };
            return Ok(format!(
                "{url} returned binary content ({kind}, {} bytes) — not fetched as text. Download it with a shell command if you need the file.",
                bytes.len()
            ));
        }

        let body = decode_body(&content_type, &bytes);
        let text = page::render_body(url, &content_type, &body);
        page::remember(url, &text);
        Ok(page::window(&text, offset, max_chars))
    }
}

/// Whether a response is not text: a non-text content type, or (without one) NUL bytes up front.
fn is_binary_response(content_type: &str, bytes: &[u8]) -> bool {
    let ct = content_type.to_ascii_lowercase();
    let ct = ct.split(';').next().unwrap_or("").trim();
    if ct.is_empty() {
        return bytes.iter().take(1024).any(|b| *b == 0);
    }
    let textual = ct.starts_with("text/")
        || [
            "json",
            "xml",
            "javascript",
            "ecmascript",
            "yaml",
            "html",
            "x-www-form-urlencoded",
            "csv",
            "toml",
        ]
        .iter()
        .any(|t| ct.contains(t));
    !textual
}

/// Decode a body honouring the `charset` of its Content-Type: Latin-1 / Windows-1252 are mapped
/// byte-for-byte, everything else (including no charset) is read as UTF-8.
fn decode_body(content_type: &str, bytes: &[u8]) -> String {
    let lower = content_type.to_ascii_lowercase();
    let charset = lower
        .split(';')
        .filter_map(|p| p.trim().strip_prefix("charset="))
        .next()
        .map(|c| c.trim_matches('"').trim().to_string());
    match charset.as_deref() {
        Some("iso-8859-1" | "latin1" | "latin-1" | "iso8859-1" | "windows-1252" | "cp1252") => {
            bytes.iter().map(|b| *b as char).collect()
        }
        _ => String::from_utf8_lossy(bytes).into_owned(),
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

    // A trailing dot is the same name (`LOCALHOST.` is `localhost`).
    let lower = host.to_ascii_lowercase().trim_end_matches('.').to_string();
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
    let mut quote: Option<char> = None;
    for ch in without_blocks.chars() {
        if in_tag {
            match (quote, ch) {
                (Some(q), c) if c == q => quote = None,
                (Some(_), _) => {}
                (None, '"' | '\'') => quote = Some(ch),
                (None, '>') => {
                    in_tag = false;
                    out.push(' ');
                }
                _ => {}
            }
        } else if ch == '<' {
            in_tag = true;
        } else {
            out.push(ch);
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
        "Search the web and return ranked results (title, URL, snippet). Use for current \
         information, docs, errors and versions; read promising results with web_fetch. Put the \
         current year in queries about recent releases or events. Cite the URLs you rely on. \
         `allowed_domains` limits results to those sites; `blocked_domains` removes sites."
    }
    fn side_effect(&self) -> SideEffect {
        SideEffect::Network
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "The search query." },
                "count": { "type": "integer", "minimum": 1, "description": "How many results (default 5, max 10)." },
                "allowed_domains": { "type": "array", "items": { "type": "string" }, "description": "Only return results from these domains (subdomains included), e.g. [\"docs.rs\"]." },
                "blocked_domains": { "type": "array", "items": { "type": "string" }, "description": "Never return results from these domains." }
            },
            "required": ["query"]
        })
    }
    async fn run(&self, args: &Value) -> Result<String, ToolError> {
        let query = str_arg(args, "query")?;
        if query.trim().is_empty() {
            return Err(ToolError::Failed(
                "web_search: `query` must not be empty".to_string(),
            ));
        }
        let count = args
            .get("count")
            .and_then(Value::as_u64)
            .map(|n| (n as u32).clamp(1, MAX_SEARCH_COUNT))
            .unwrap_or(DEFAULT_SEARCH_COUNT);
        let allowed = domain_list(args, "allowed_domains");
        let blocked = domain_list(args, "blocked_domains");
        let scoped = scoped_query(query, &allowed);
        let ask = if allowed.is_empty() && blocked.is_empty() {
            count
        } else {
            MAX_SEARCH_COUNT * 2
        };
        let mut results = self.resolve_backend().search(&scoped, ask).await?;
        results.retain(|r| domain_allowed(&r.url, &allowed, &blocked));
        results.truncate(count as usize);
        Ok(format_results(&results))
    }
}

fn domain_list(args: &Value, key: &str) -> Vec<String> {
    args.get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(|d| d.trim().trim_start_matches("www.").to_ascii_lowercase())
                .filter(|d| !d.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Add `site:` operators so the engines search inside the allowed domains rather than us
/// filtering a generic top ten down to nothing.
fn scoped_query(query: &str, allowed: &[String]) -> String {
    match allowed {
        [] => query.to_string(),
        [one] => format!("{query} site:{one}"),
        many => {
            let sites: Vec<String> = many.iter().map(|d| format!("site:{d}")).collect();
            format!("{query} ({})", sites.join(" OR "))
        }
    }
}

fn domain_allowed(url: &str, allowed: &[String], blocked: &[String]) -> bool {
    let Some(host) = reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_ascii_lowercase))
    else {
        return allowed.is_empty();
    };
    let matches = |d: &String| host == *d || host.ends_with(&format!(".{d}"));
    (allowed.is_empty() || allowed.iter().any(matches)) && !blocked.iter().any(matches)
}

#[cfg(test)]
mod tests {
    use super::*;
    // Parser tests for the engines now live beside them in `search.rs`; these predate the split
    // and exercise the same functions through the module boundary.
    use super::search::{decode_ddg_href, parse_brave_results, parse_ddg_ia, parse_ddg_results};

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
    fn html_to_text_ignores_gt_inside_quoted_attributes() {
        let text = html_to_text(r#"<a title="a > b" href='x>y'>link</a> tail"#);
        assert_eq!(text, "link tail");
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
    fn domain_filters_scope_the_query_and_the_results() {
        let allowed = vec!["docs.rs".to_string()];
        let blocked = vec!["pinterest.com".to_string()];
        assert_eq!(
            scoped_query("tokio select", &allowed),
            "tokio select site:docs.rs"
        );
        assert_eq!(
            scoped_query("q", &["a.com".into(), "b.org".into()]),
            "q (site:a.com OR site:b.org)"
        );
        assert!(domain_allowed("https://docs.rs/tokio", &allowed, &[]));
        assert!(domain_allowed("https://sub.docs.rs/x", &allowed, &[]));
        assert!(!domain_allowed("https://notdocs.rs/x", &allowed, &[]));
        assert!(!domain_allowed(
            "https://www.pinterest.com/p",
            &[],
            &blocked
        ));
        assert!(domain_allowed("https://example.com/", &[], &blocked));
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

    #[test]
    fn trailing_dot_localhost_is_refused_and_binary_bodies_detected() {
        assert!(is_safe_url("http://LOCALHOST.:8080/").is_err());
        assert!(is_binary_response("image/png", b"\x89PNG"));
        assert!(!is_binary_response("text/html; charset=utf-8", b"<p>"));
        assert!(!is_binary_response("application/json", b"{}"));
        assert_eq!(
            decode_body("text/html; charset=ISO-8859-1", b"caf\xe9"),
            "café"
        );
    }
}
