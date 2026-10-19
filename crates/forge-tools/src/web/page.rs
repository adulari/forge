//! What `web_fetch` hands the model: a page rendered as Markdown, windowed, and cached.
//!
//! The first cut flattened every page to one line of text. Headings, paragraphs, links and code
//! blocks all disappeared, and the 10k-char window was mostly navigation chrome, so a docs page
//! often ran out before its first code sample. Rendering the main region as Markdown keeps the
//! structure the model needs, and paging through a cached copy lets it read past the window
//! without fetching again.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::html_to_text;

const CACHE_TTL: Duration = Duration::from_secs(15 * 60);
const CACHE_ENTRIES: usize = 32;

/// Elements that are never page content.
const ALWAYS_SKIP: [&str; 10] = [
    "script", "style", "noscript", "svg", "template", "iframe", "form", "button", "select", "nav",
];
/// Page chrome. Only skipped when no `<main>`/`<article>` was found, because inside an article a
/// `<header>` usually carries the title.
const CHROME: [&str; 3] = ["header", "footer", "aside"];

/// Render a fetched body for the model. HTML becomes Markdown; any other text (plain text, JSON,
/// source files, Markdown) passes through untouched, so `Vec<String>` in a raw `.rs` file is not
/// eaten as a tag.
pub(super) fn render_body(url: &str, content_type: &str, body: &str) -> String {
    if is_html(content_type, body) {
        match reqwest::Url::parse(url) {
            Ok(base) => html_to_markdown(&absolutize_links(body, &base)),
            Err(_) => html_to_markdown(body),
        }
    } else {
        body.trim().to_string()
    }
}

fn is_html(content_type: &str, body: &str) -> bool {
    let ct = content_type.to_ascii_lowercase();
    if ct.contains("html") {
        return true;
    }
    if !ct.is_empty() {
        return false;
    }
    let head: String = body
        .trim_start()
        .chars()
        .take(64)
        .collect::<String>()
        .to_ascii_lowercase();
    head.starts_with("<!doctype html") || head.starts_with("<html")
}

pub(crate) fn html_to_markdown(html: &str) -> String {
    let title = super::extract_title(html);
    let (region, focused) = main_region(html);
    let mut skip: Vec<&str> = ALWAYS_SKIP.to_vec();
    if !focused {
        skip.extend(CHROME);
    }
    let converter = htmd::HtmlToMarkdown::builder()
        .options(htmd::options::Options {
            ul_bullet_spacing: 1,
            bullet_list_marker: htmd::options::BulletListMarker::Dash,
            ..Default::default()
        })
        .skip_tags(skip)
        .build();
    let markdown = match converter.convert(region) {
        Ok(md) => tidy(&md),
        Err(_) => html_to_text(html),
    };
    match title {
        Some(t) if !markdown.contains(&t) => format!("# {t}\n\n{markdown}"),
        _ => markdown,
    }
}

/// Rewrite relative `href` values to absolute URLs, so a link in the Markdown can be passed
/// straight back to `web_fetch`. Fragments and non-http schemes are left alone.
fn absolutize_links(html: &str, base: &reqwest::Url) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len() + 256);
    let mut cursor = 0;
    while let Some(rel) = lower[cursor..].find("href=") {
        let value_start = cursor + rel + "href=".len();
        let Some(quote) = html[value_start..]
            .chars()
            .next()
            .filter(|c| *c == '"' || *c == '\'')
        else {
            out.push_str(&html[cursor..value_start]);
            cursor = value_start;
            continue;
        };
        let inner = value_start + 1;
        let Some(len) = html[inner..].find(quote) else {
            break;
        };
        let value = &html[inner..inner + len];
        out.push_str(&html[cursor..inner]);
        let has_scheme = value.split_once(':').is_some_and(|(scheme, _)| {
            !scheme.is_empty()
                && scheme
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
        });
        match base.join(value) {
            Ok(abs) if !value.starts_with('#') && !has_scheme => out.push_str(abs.as_str()),
            _ => out.push_str(value),
        }
        cursor = inner + len;
    }
    out.push_str(&html[cursor..]);
    out
}

/// The `<main>` or `<article>` element when the page has one, else the whole document. Returns
/// whether a content element was found.
fn main_region(html: &str) -> (&str, bool) {
    let lower = html.to_ascii_lowercase();
    for tag in ["main", "article"] {
        let open = format!("<{tag}");
        let close = format!("</{tag}>");
        let Some(start) = find_open_tag(&lower, &open) else {
            continue;
        };
        if let Some(end) = lower.rfind(&close) {
            if end > start {
                return (&html[start..end + close.len()], true);
            }
        }
    }
    (html, false)
}

/// The first `<tag` followed by `>` or whitespace, so `<main` does not match `<mainframe>`.
fn find_open_tag(lower: &str, open: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(rel) = lower[from..].find(open) {
        let at = from + rel;
        match lower.as_bytes().get(at + open.len()) {
            Some(b'>') | Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r') => return Some(at),
            _ => from = at + open.len(),
        }
    }
    None
}

/// Trim trailing spaces and squeeze runs of blank lines to one.
fn tidy(md: &str) -> String {
    let mut out = String::with_capacity(md.len());
    let mut blank = 0;
    for line in md.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(line);
        out.push('\n');
    }
    out.trim().to_string()
}

/// The slice of `text` starting at char `offset`, at most `max` chars, with a footer telling the
/// model how to read on when more remains.
pub(super) fn window(text: &str, offset: usize, max: usize) -> String {
    let total = text.chars().count();
    if offset >= total && total > 0 {
        return format!("[offset {offset} is past the end; the page has {total} characters]");
    }
    let end = offset.saturating_add(max).min(total);
    let slice: String = text.chars().skip(offset).take(end - offset).collect();
    if offset == 0 && end == total {
        return slice;
    }
    let mut out = slice;
    if end < total {
        out.push_str(&format!(
            "\n\n[showing characters {offset}-{end} of {total}. Call web_fetch again with the same \
             url and offset={end} to read more.]"
        ));
    } else {
        out.push_str(&format!(
            "\n\n[end of page: characters {offset}-{end} of {total}]"
        ));
    }
    out
}

type Cache = Mutex<HashMap<String, (Instant, String)>>;

fn cache() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(super) fn cached(url: &str) -> Option<String> {
    let map = cache().lock().ok()?;
    map.get(url)
        .filter(|(at, _)| at.elapsed() < CACHE_TTL)
        .map(|(_, text)| text.clone())
}

pub(super) fn remember(url: &str, text: &str) {
    let Ok(mut map) = cache().lock() else {
        return;
    };
    map.retain(|_, (at, _)| at.elapsed() < CACHE_TTL);
    if map.len() >= CACHE_ENTRIES {
        if let Some(oldest) = map
            .iter()
            .min_by_key(|(_, (at, _))| *at)
            .map(|(k, _)| k.clone())
        {
            map.remove(&oldest);
        }
    }
    map.insert(url.to_string(), (Instant::now(), text.to_string()));
}

/// Whether two URLs are on the same site, treating `www.` as the same host.
pub(super) fn same_site(a: &reqwest::Url, b: &reqwest::Url) -> bool {
    fn bare(u: &reqwest::Url) -> String {
        let host = u.host_str().unwrap_or("").to_ascii_lowercase();
        host.strip_prefix("www.")
            .map(str::to_string)
            .unwrap_or(host)
    }
    bare(a) == bare(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_keeps_structure_and_drops_chrome() {
        let html = r#"<!doctype html><html><head><title>Tokio select</title>
            <script>var x = 1;</script></head><body>
            <nav><a href="/">Home</a> <a href="/blog">Blog</a></nav>
            <main><h1>select!</h1><p>Waits on <a href="https://docs.rs/tokio">multiple</a> futures.</p>
            <pre><code>tokio::select! { _ = a => {} }</code></pre>
            <ul><li>one</li><li>two</li></ul></main>
            <footer>Copyright</footer></body></html>"#;
        let md = html_to_markdown(html);
        assert!(md.starts_with("# Tokio select"), "{md}");
        assert!(md.contains("# select!"), "{md}");
        assert!(md.contains("[multiple](https://docs.rs/tokio)"), "{md}");
        assert!(
            md.contains("```\ntokio::select! { _ = a => {} }\n```"),
            "{md}"
        );
        assert!(md.contains("- one\n- two"), "{md}");
        for gone in ["Home", "Blog", "Copyright", "var x"] {
            assert!(!md.contains(gone), "{gone} leaked: {md}");
        }
    }

    #[test]
    fn without_a_main_element_header_and_footer_are_dropped() {
        let html = "<html><body><header>Site menu</header><h2>Body</h2><p>Text</p>\
                    <footer>Legal</footer></body></html>";
        let md = html_to_markdown(html);
        assert!(md.contains("## Body") && md.contains("Text"), "{md}");
        assert!(!md.contains("Site menu") && !md.contains("Legal"), "{md}");
    }

    #[test]
    fn non_html_bodies_pass_through_with_angle_brackets_intact() {
        let rust = "fn f() -> Vec<String> { vec![] }";
        let u = "https://example.com/f.rs";
        assert_eq!(render_body(u, "text/plain; charset=utf-8", rust), rust);
        assert_eq!(
            render_body(u, "application/json", r#"{"a":"<b>"}"#),
            r#"{"a":"<b>"}"#
        );
        assert_eq!(render_body(u, "", rust), rust);
        assert!(render_body(u, "", "<!DOCTYPE html><p>hi</p>").contains("hi"));
    }

    #[test]
    fn relative_links_become_absolute() {
        let base =
            reqwest::Url::parse("https://docs.rs/tokio/latest/tokio/macro.select.html").unwrap();
        let html = r##"<a href="../src/select.rs.html#5">Source</a> <a href='index.html'>Home</a>
            <a href="#anchor">Here</a> <a href="mailto:a@b.c">Mail</a> <a href="https://x.org/">X</a>"##;
        let out = absolutize_links(html, &base);
        assert!(
            out.contains(r#"href="https://docs.rs/tokio/latest/src/select.rs.html#5""#),
            "{out}"
        );
        assert!(
            out.contains("href='https://docs.rs/tokio/latest/tokio/index.html'"),
            "{out}"
        );
        assert!(
            out.contains(r##"href="#anchor""##) && out.contains("mailto:a@b.c"),
            "{out}"
        );
        assert!(out.contains(r#"href="https://x.org/""#), "{out}");
    }

    #[test]
    fn mainframe_is_not_main() {
        assert_eq!(
            find_open_tag("<mainframe><main class=x>", "<main"),
            Some(11)
        );
    }

    #[test]
    fn window_pages_with_a_continuation_footer() {
        let text = "abcdefghij";
        assert_eq!(window(text, 0, 20), text);
        let first = window(text, 0, 4);
        assert!(
            first.starts_with("abcd") && first.contains("offset=4"),
            "{first}"
        );
        let last = window(text, 8, 4);
        assert!(
            last.starts_with("ij") && last.contains("end of page"),
            "{last}"
        );
        assert!(window(text, 50, 4).contains("past the end"));
    }

    #[test]
    fn cache_returns_what_was_remembered() {
        remember("https://example.test/a", "page a");
        assert_eq!(cached("https://example.test/a").as_deref(), Some("page a"));
        assert_eq!(cached("https://example.test/missing"), None);
    }

    #[test]
    fn www_is_the_same_site() {
        let a = reqwest::Url::parse("https://example.com/x").unwrap();
        let b = reqwest::Url::parse("https://www.example.com/y").unwrap();
        let c = reqwest::Url::parse("https://evil.example.net/").unwrap();
        assert!(same_site(&a, &b));
        assert!(!same_site(&a, &c));
    }
}
