//! Rendering the evidence ledger into the `### Evidence` section of a PR body.

use std::collections::HashMap;
use std::path::Path;

use super::{EvidenceItem, EvidenceKind, Ledger};

/// What the renderer knows beyond the manifest. The default renders the manifest alone, which is
/// what `/pr`'s preview of the block shows; `open_pr` fills both fields in.
#[derive(Debug, Default, Clone, Copy)]
pub struct PrBlockOptions<'a> {
    /// Published image URL per screenshot file name (the `forge-evidence` branch), so the image
    /// renders inline in the PR instead of being named by a path only this machine has.
    pub image_urls: Option<&'a HashMap<String, String>>,
    /// Preview URLs whose tunnel is still up. `None` = liveness unknown: every recorded preview is
    /// listed. With a set, closed tunnels are left out and only counted, since a dead link in a PR
    /// reads as a broken deploy.
    pub live_previews: Option<&'a [String]>,
}

/// The evidence section for `session`, or `None` when it captured nothing.
pub fn render(workspace: &Path, session: &str, options: &PrBlockOptions<'_>) -> Option<String> {
    render_ledger(&Ledger::new(workspace, session)?, options)
}

impl Ledger {
    /// [`render`] for this ledger's session.
    pub fn pr_block(&self, options: &PrBlockOptions<'_>) -> Option<String> {
        render_ledger(self, options)
    }
}

fn render_ledger(ledger: &Ledger, options: &PrBlockOptions<'_>) -> Option<String> {
    let session = ledger.dir().file_name()?.to_str()?;
    let items = ledger.items();
    if items.is_empty() {
        return None;
    }
    let rel_dir = format!(".forge/evidence/{session}");
    let mut block = String::from("### Evidence (captured, not reviewed)\n");
    let of_kind = |kind: EvidenceKind| -> Vec<&EvidenceItem> {
        items.iter().filter(|item| item.kind == kind).collect()
    };
    let tests = of_kind(EvidenceKind::TestOutput);
    if !tests.is_empty() {
        block.push_str("Checks that passed during the session:\n");
        for item in tests {
            block.push_str(&format!(
                "- `{}` — exit {} at {} (`{rel_dir}/{}`, sha256 `{}`)\n",
                display_command(item.command.as_deref().unwrap_or("?")),
                item.exit_code.unwrap_or(0),
                item.ts,
                item.file,
                short_hash(&item.sha256),
            ));
        }
    }
    let shots = of_kind(EvidenceKind::Screenshot);
    if !shots.is_empty() {
        block.push_str("Screenshots:\n");
        for item in shots {
            block.push_str(&screenshot_line(item, &rel_dir, options.image_urls));
        }
    }
    push_previews(&mut block, &of_kind(EvidenceKind::Preview), options);
    Some(block)
}

fn screenshot_line(
    item: &EvidenceItem,
    rel_dir: &str,
    image_urls: Option<&HashMap<String, String>>,
) -> String {
    let mut line = format!("- `{rel_dir}/{}` ({}", item.file, item.source);
    if let Some(url) = &item.url {
        line.push_str(&format!(", {url}"));
    }
    line.push_str(&format!(", sha256 `{}`)", short_hash(&item.sha256)));
    if let Some(caption) = &item.caption {
        line.push_str(&format!(" — {caption}"));
    }
    line.push('\n');
    if let Some(image) = image_urls.and_then(|urls| urls.get(&item.file)) {
        let alt = item.caption.as_deref().unwrap_or(&item.file);
        line.push_str(&format!("  ![{}]({image})\n", alt.replace(['[', ']'], "")));
    }
    line
}

fn push_previews(block: &mut String, previews: &[&EvidenceItem], options: &PrBlockOptions<'_>) {
    let mut seen: Vec<&str> = Vec::new();
    let mut closed = 0usize;
    let mut lines = String::new();
    for item in previews {
        let Some(url) = item.url.as_deref() else {
            continue;
        };
        if seen.contains(&url) {
            continue;
        }
        seen.push(url);
        if options
            .live_previews
            .is_some_and(|live| !live.iter().any(|l| l == url))
        {
            closed += 1;
            continue;
        }
        let what = item.caption.as_deref().unwrap_or("dev server");
        lines.push_str(&format!("- {url} ({what}, opened {})\n", item.ts));
    }
    if lines.is_empty() && closed == 0 {
        return;
    }
    block.push_str(
        "Preview URLs (temporary public tunnels; they close with the dev server or after their \
         TTL):\n",
    );
    block.push_str(&lines);
    if closed > 0 {
        block.push_str(&format!(
            "- {closed} earlier preview URL(s) from this session have since closed\n"
        ));
    }
}

/// The command as shown in a PR body, which is public: values of leading `NAME=value` env
/// assignments (`API_KEY=… cargo test`) are masked, and backticks cannot break the code span.
fn display_command(command: &str) -> String {
    let mut words = command.split_whitespace().peekable();
    let mut shown: Vec<String> = Vec::new();
    while let Some(word) = words.peek() {
        let Some((name, _)) = word.split_once('=') else {
            break;
        };
        let is_name = !name.is_empty()
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && !name.starts_with(|c: char| c.is_ascii_digit());
        if !is_name {
            break;
        }
        shown.push(format!("{name}=***"));
        words.next();
    }
    shown.extend(words.map(str::to_string));
    shown.join(" ").replace('`', "'")
}

fn short_hash(hash: &str) -> &str {
    &hash[..hash.len().min(12)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ledger() -> (tempfile::TempDir, Ledger) {
        let dir = tempfile::tempdir().unwrap();
        let ledger = Ledger::new(dir.path(), "sess-p").unwrap();
        (dir, ledger)
    }

    #[test]
    fn published_screenshots_render_inline() {
        let (dir, ledger) = ledger();
        let (_, item) = ledger
            .record(
                EvidenceKind::Screenshot,
                "browser",
                "png",
                b"png",
                super::super::Capture {
                    caption: Some("cart [empty]"),
                    ..Default::default()
                },
            )
            .unwrap();
        let urls = HashMap::from([(item.file.clone(), "https://img/1.png".to_string())]);
        let options = PrBlockOptions {
            image_urls: Some(&urls),
            live_previews: None,
        };
        let block = render(dir.path(), "sess-p", &options).unwrap();
        assert!(
            block.contains("— cart [empty]\n  ![cart empty](https://img/1.png)\n"),
            "{block}"
        );
        let plain = render(dir.path(), "sess-p", &PrBlockOptions::default()).unwrap();
        assert!(!plain.contains("!["), "{plain}");
    }

    #[test]
    fn previews_are_listed_once_and_closed_ones_only_counted() {
        let (dir, ledger) = ledger();
        ledger
            .record_preview("https://a.trycloudflare.com", 41, 5173)
            .unwrap();
        ledger
            .record_preview("https://a.trycloudflare.com", 41, 5173)
            .unwrap();
        ledger
            .record_preview("https://b.trycloudflare.com", 42, 3000)
            .unwrap();

        let all = render(dir.path(), "sess-p", &PrBlockOptions::default()).unwrap();
        assert_eq!(all.matches("https://a.trycloudflare.com").count(), 1);
        assert!(all.contains("- https://b.trycloudflare.com (job 42 port 3000, opened "));
        assert!(!all.contains("closed\n"), "{all}");

        let live = vec!["https://b.trycloudflare.com".to_string()];
        let options = PrBlockOptions {
            image_urls: None,
            live_previews: Some(&live),
        };
        let filtered = render(dir.path(), "sess-p", &options).unwrap();
        assert!(
            !filtered.contains("https://a.trycloudflare.com"),
            "{filtered}"
        );
        assert!(filtered.contains("https://b.trycloudflare.com"));
        assert!(filtered.contains("- 1 earlier preview URL(s) from this session have since closed"));
    }
}
