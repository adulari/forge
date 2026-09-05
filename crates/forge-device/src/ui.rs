//! The on-screen view hierarchy: parse a `uiautomator dump`, then find things in it.
//!
//! This is what makes app testing possible without a human looking at the screen. A screenshot
//! costs a vision round-trip and still can't be clicked precisely; the hierarchy gives exact
//! bounds for every element, so "tap the Log in button" becomes a lookup rather than a guess.
//!
//! The XML is parsed by hand rather than with a dependency: uiautomator emits one fixed,
//! namespace-free shape, and a 100-line scanner for it is less risk than another crate.

use serde::Serialize;

/// A rectangle in screen pixels, as `bounds="[l,t][r,b]"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
pub struct Bounds {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Bounds {
    /// The tap point for this element.
    pub fn center(&self) -> (i32, i32) {
        ((self.left + self.right) / 2, (self.top + self.bottom) / 2)
    }

    pub fn width(&self) -> i32 {
        self.right - self.left
    }

    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }

    /// A zero-area element cannot be tapped, and uiautomator emits plenty of them.
    pub fn is_empty(&self) -> bool {
        self.width() <= 0 || self.height() <= 0
    }

    fn parse(raw: &str) -> Option<Self> {
        let (first, second) = raw.split_once("][")?;
        let mut left_top = first.trim_start_matches('[').split(',');
        let mut right_bottom = second.trim_end_matches(']').split(',');
        Some(Self {
            left: left_top.next()?.trim().parse().ok()?,
            top: left_top.next()?.trim().parse().ok()?,
            right: right_bottom.next()?.trim().parse().ok()?,
            bottom: right_bottom.next()?.trim().parse().ok()?,
        })
    }
}

/// One node of the view hierarchy, flattened with its depth.
#[derive(Debug, Clone, Serialize, Default)]
pub struct UiNode {
    pub class: String,
    pub text: String,
    pub resource_id: String,
    pub content_desc: String,
    pub package: String,
    pub bounds: Bounds,
    pub clickable: bool,
    pub long_clickable: bool,
    pub scrollable: bool,
    pub checkable: bool,
    pub checked: bool,
    pub selected: bool,
    pub focused: bool,
    pub enabled: bool,
    pub password: bool,
    #[serde(skip)]
    pub depth: usize,
}

impl UiNode {
    /// The short form of `resource-id`: `com.app:id/login` → `login`.
    pub fn id_suffix(&self) -> &str {
        self.resource_id.rsplit('/').next().unwrap_or("")
    }

    /// Worth showing in a compact dump: something a caller could act on or read.
    pub fn is_interesting(&self) -> bool {
        !self.bounds.is_empty()
            && (self.clickable
                || self.scrollable
                || self.checkable
                || self.long_clickable
                || !self.text.is_empty()
                || !self.content_desc.is_empty()
                || !self.resource_id.is_empty())
    }

    /// A one-line human/model-readable description.
    pub fn describe(&self) -> String {
        let mut parts = vec![short_class(&self.class).to_string()];
        if !self.text.is_empty() {
            parts.push(format!("{:?}", self.text));
        }
        if !self.content_desc.is_empty() {
            parts.push(format!("desc={:?}", self.content_desc));
        }
        if !self.resource_id.is_empty() {
            parts.push(format!("id={}", self.id_suffix()));
        }
        let mut flags = Vec::new();
        if self.clickable {
            flags.push("clickable");
        }
        if self.scrollable {
            flags.push("scrollable");
        }
        if self.checkable {
            flags.push(if self.checked { "checked" } else { "unchecked" });
        }
        if self.password {
            flags.push("password");
        }
        if !self.enabled {
            flags.push("disabled");
        }
        if !flags.is_empty() {
            parts.push(format!("[{}]", flags.join(",")));
        }
        let (x, y) = self.bounds.center();
        parts.push(format!("@{x},{y}"));
        parts.join(" ")
    }
}

fn short_class(class: &str) -> &str {
    class.rsplit('.').next().unwrap_or(class)
}

/// What to look for. Every set field must match; unset fields are ignored.
#[derive(Debug, Clone, Default)]
pub struct Selector {
    pub text: Option<String>,
    pub resource_id: Option<String>,
    pub content_desc: Option<String>,
    pub class: Option<String>,
    /// Match `text`/`content_desc` exactly rather than as a case-insensitive substring.
    pub exact: bool,
    /// Only consider nodes that can actually be tapped.
    pub clickable_only: bool,
}

impl Selector {
    pub fn is_empty(&self) -> bool {
        self.text.is_none()
            && self.resource_id.is_none()
            && self.content_desc.is_none()
            && self.class.is_none()
    }

    pub fn matches(&self, node: &UiNode) -> bool {
        if self.clickable_only && !node.clickable && !node.long_clickable {
            return false;
        }
        if let Some(want) = &self.text {
            if !string_match(&node.text, want, self.exact) {
                return false;
            }
        }
        if let Some(want) = &self.content_desc {
            if !string_match(&node.content_desc, want, self.exact) {
                return false;
            }
        }
        if let Some(want) = &self.resource_id {
            // Suffix match: callers know the id as `login_button`, not the package-qualified form.
            let full = node.resource_id.as_str();
            if !(full == want || node.id_suffix() == want || full.ends_with(want)) {
                return false;
            }
        }
        if let Some(want) = &self.class {
            if !node.class.to_lowercase().contains(&want.to_lowercase()) {
                return false;
            }
        }
        true
    }
}

fn string_match(actual: &str, want: &str, exact: bool) -> bool {
    if exact {
        actual == want
    } else {
        actual.to_lowercase().contains(&want.to_lowercase())
    }
}

/// A parsed hierarchy.
#[derive(Debug, Clone, Default)]
pub struct Hierarchy {
    pub nodes: Vec<UiNode>,
}

impl Hierarchy {
    /// Parse a `uiautomator dump` document. Malformed fragments are skipped rather than fatal:
    /// a partial hierarchy is still useful, and dumps can be truncated mid-animation.
    pub fn parse(xml: &str) -> Self {
        let mut nodes = Vec::new();
        let mut depth = 0usize;
        let bytes = xml.as_bytes();
        let mut cursor = 0usize;
        while let Some(start) = find(bytes, cursor, b'<') {
            let Some(end) = find(bytes, start + 1, b'>') else {
                break;
            };
            let tag = &xml[start + 1..end];
            cursor = end + 1;
            if tag.starts_with('?') || tag.starts_with('!') {
                continue;
            }
            if let Some(name) = tag.strip_prefix('/') {
                if name.trim() == "node" {
                    depth = depth.saturating_sub(1);
                }
                continue;
            }
            let self_closing = tag.ends_with('/');
            let body = tag.trim_end_matches('/');
            if !body.starts_with("node") {
                continue;
            }
            let mut node = UiNode {
                depth,
                ..Default::default()
            };
            for (key, value) in attributes(body) {
                match key {
                    "class" => node.class = value,
                    "text" => node.text = value,
                    "resource-id" => node.resource_id = value,
                    "content-desc" => node.content_desc = value,
                    "package" => node.package = value,
                    "bounds" => node.bounds = Bounds::parse(&value).unwrap_or_default(),
                    "clickable" => node.clickable = value == "true",
                    "long-clickable" => node.long_clickable = value == "true",
                    "scrollable" => node.scrollable = value == "true",
                    "checkable" => node.checkable = value == "true",
                    "checked" => node.checked = value == "true",
                    "selected" => node.selected = value == "true",
                    "focused" => node.focused = value == "true",
                    "enabled" => node.enabled = value == "true",
                    "password" => node.password = value == "true",
                    _ => {}
                }
            }
            nodes.push(node);
            if !self_closing {
                depth += 1;
            }
        }
        Self { nodes }
    }

    /// Every node matching the selector, in document order.
    pub fn find(&self, selector: &Selector) -> Vec<&UiNode> {
        self.nodes
            .iter()
            .filter(|node| selector.matches(node))
            .collect()
    }

    /// The nodes worth showing to a caller, and how many were hidden.
    pub fn interesting(&self) -> (Vec<&UiNode>, usize) {
        let kept: Vec<_> = self
            .nodes
            .iter()
            .filter(|node| node.is_interesting())
            .collect();
        (kept.clone(), self.nodes.len() - kept.len())
    }

    /// The app the screen belongs to, by majority of nodes.
    pub fn package(&self) -> Option<&str> {
        self.nodes
            .iter()
            .map(|n| n.package.as_str())
            .find(|p| !p.is_empty())
    }
}

/// Render nodes as an indented outline.
pub fn outline(nodes: &[&UiNode]) -> String {
    nodes
        .iter()
        .map(|node| format!("{}{}", "  ".repeat(node.depth.min(12)), node.describe()))
        .collect::<Vec<_>>()
        .join("\n")
}

fn find(bytes: &[u8], from: usize, needle: u8) -> Option<usize> {
    bytes
        .get(from..)?
        .iter()
        .position(|b| *b == needle)
        .map(|offset| from + offset)
}

/// Split `node key="value" key="value"` into pairs, decoding XML entities.
fn attributes(body: &str) -> Vec<(&str, String)> {
    let mut pairs = Vec::new();
    let bytes = body.as_bytes();
    let mut cursor = 0usize;
    while let Some(equals) = find(bytes, cursor, b'=') {
        let key_start = body[..equals]
            .rfind(|c: char| c.is_whitespace())
            .map(|index| index + 1)
            .unwrap_or(0);
        let key = body[key_start..equals].trim();
        let Some(open) = find(bytes, equals + 1, b'"') else {
            break;
        };
        let Some(close) = find(bytes, open + 1, b'"') else {
            break;
        };
        if !key.is_empty() {
            pairs.push((key, unescape(&body[open + 1..close])));
        }
        cursor = close + 1;
    }
    pairs
}

fn unescape(raw: &str) -> String {
    if !raw.contains('&') {
        return raw.to_string();
    }
    raw.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#10;", "\n")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    const DUMP: &str = r#"<?xml version='1.0' encoding='UTF-8' standalone='yes' ?>
<hierarchy rotation="0">
  <node index="0" text="" resource-id="" class="android.widget.FrameLayout" package="com.app" content-desc="" checkable="false" checked="false" clickable="false" enabled="true" focusable="false" focused="false" scrollable="false" long-clickable="false" password="false" selected="false" bounds="[0,0][1080,2400]">
    <node index="0" text="Email &amp; phone" resource-id="com.app:id/email" class="android.widget.EditText" package="com.app" content-desc="" checkable="false" checked="false" clickable="true" enabled="true" focusable="true" focused="false" scrollable="false" long-clickable="true" password="false" selected="false" bounds="[100,500][980,620]" />
    <node index="1" text="Log in" resource-id="com.app:id/login_button" class="android.widget.Button" package="com.app" content-desc="Log in to your account" checkable="false" checked="false" clickable="true" enabled="true" focusable="true" focused="false" scrollable="false" long-clickable="false" password="false" selected="false" bounds="[100,700][980,820]" />
    <node index="2" text="" resource-id="" class="android.view.View" package="com.app" content-desc="" checkable="false" checked="false" clickable="false" enabled="true" focusable="false" focused="false" scrollable="false" long-clickable="false" password="false" selected="false" bounds="[0,0][0,0]" />
  </node>
</hierarchy>"#;

    #[test]
    fn parses_nodes_bounds_and_entities() {
        let tree = Hierarchy::parse(DUMP);
        assert_eq!(tree.nodes.len(), 4);
        assert_eq!(tree.nodes[1].text, "Email & phone");
        assert_eq!(tree.nodes[1].bounds.center(), (540, 560));
        assert_eq!(tree.nodes[2].id_suffix(), "login_button");
        assert_eq!(tree.package(), Some("com.app"));
    }

    #[test]
    fn tracks_depth_across_self_closing_and_nested_nodes() {
        let tree = Hierarchy::parse(DUMP);
        assert_eq!(tree.nodes[0].depth, 0);
        assert!(tree.nodes[1..].iter().all(|node| node.depth == 1));
    }

    #[test]
    fn finds_by_partial_text_and_by_bare_resource_id() {
        let tree = Hierarchy::parse(DUMP);
        let by_text = tree.find(&Selector {
            text: Some("log in".into()),
            ..Default::default()
        });
        // Matches the button's text and, via content-desc, nothing else: text is text.
        assert_eq!(by_text.len(), 1);
        assert_eq!(by_text[0].id_suffix(), "login_button");

        let by_id = tree.find(&Selector {
            resource_id: Some("login_button".into()),
            ..Default::default()
        });
        assert_eq!(by_id.len(), 1);
        assert_eq!(by_id[0].bounds.center(), (540, 760));
    }

    #[test]
    fn exact_text_does_not_match_a_substring() {
        let tree = Hierarchy::parse(DUMP);
        let loose = Selector {
            text: Some("Log".into()),
            ..Default::default()
        };
        let strict = Selector {
            text: Some("Log".into()),
            exact: true,
            ..Default::default()
        };
        assert_eq!(tree.find(&loose).len(), 1);
        assert!(tree.find(&strict).is_empty());
    }

    #[test]
    fn zero_area_and_featureless_nodes_are_dropped_from_a_compact_dump() {
        let tree = Hierarchy::parse(DUMP);
        let (kept, hidden) = tree.interesting();
        assert_eq!(
            hidden, 2,
            "the root frame and the empty view carry nothing actionable"
        );
        assert_eq!(kept.len(), 2);
        assert!(outline(&kept).contains("EditText"));
    }

    #[test]
    fn a_truncated_dump_still_yields_the_nodes_it_did_contain() {
        let truncated = &DUMP[..DUMP.len() / 2];
        assert!(!Hierarchy::parse(truncated).nodes.is_empty());
    }
}
