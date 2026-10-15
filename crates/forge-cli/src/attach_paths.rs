//! File paths arriving as input: a terminal drag-and-drop or paste of a path, `/image <path>`,
//! and the path part of `@path` all go through the same unquoting and classification here.

use std::path::{Path, PathBuf};

/// Undo the quoting a terminal or shell applied to a path: surrounding quotes, `file://` URIs
/// (percent-decoded), backslash-escaped characters, and a leading `~`.
pub(crate) fn normalize_path_arg(raw: &str) -> PathBuf {
    let mut s = raw.trim().to_string();
    for q in ['"', '\''] {
        if s.len() >= 2 && s.starts_with(q) && s.ends_with(q) {
            s = s[1..s.len() - 1].to_string();
            break;
        }
    }
    if let Some(rest) = s.strip_prefix("file://") {
        let rest = rest.strip_prefix("localhost").unwrap_or(rest);
        s = percent_decode(rest);
    } else if cfg!(unix) {
        s = unescape_backslashes(&s);
    }
    expand_tilde(&s)
}

fn expand_tilde(s: &str) -> PathBuf {
    let home = || {
        std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from)
    };
    if s == "~" {
        return home().unwrap_or_else(|| PathBuf::from(s));
    }
    if let Some(rest) = s.strip_prefix("~/") {
        if let Some(h) = home() {
            return h.join(rest);
        }
    }
    PathBuf::from(s)
}

fn unescape_backslashes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some(n) => out.push(n),
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |x: u8| (x as char).to_digit(16).map(|d| d as u8);
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Split pasted text the way a shell would: whitespace separates tokens, quotes and backslashes
/// group. Quoting is kept in each token so [`normalize_path_arg`] stays the single unquoting
/// step.
pub(crate) fn split_path_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match quote {
            Some(q) => {
                cur.push(c);
                if c == q {
                    quote = None;
                }
            }
            None if c == '\\' && cfg!(unix) => {
                cur.push(c);
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                cur.push(c);
            }
            None if c.is_whitespace() => {
                if !cur.is_empty() {
                    tokens.push(std::mem::take(&mut cur));
                }
            }
            None => cur.push(c),
        }
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    tokens
}

fn looks_like_path(token: &str) -> bool {
    let t = token.trim_matches(['"', '\'']);
    t.starts_with('/')
        || t.starts_with("~/")
        || t == "~"
        || t.starts_with("./")
        || t.starts_with("../")
        || t.starts_with("file://")
        || t.starts_with("\\\\")
        || (t.len() > 2 && t.as_bytes()[0].is_ascii_alphabetic() && &t[1..3] == ":\\")
}

/// A path that names a file on someone's laptop: a home-directory location that is not here.
fn looks_foreign(path: &Path) -> bool {
    let s = path.to_string_lossy();
    let under = |prefix: &str| s.strip_prefix(prefix).is_some_and(|r| r.contains('/'));
    under("/home/")
        || under("/Users/")
        || s.starts_with("/mnt/c/Users/")
        || (s.len() > 9 && s[1..].starts_with(":\\Users\\"))
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Dropped {
    Image(PathBuf),
    Text(PathBuf),
    Pdf(PathBuf),
    Other(PathBuf),
    /// Looks like a path from another machine and does not exist here.
    Foreign(String),
}

fn classify_file(path: &Path) -> Dropped {
    use std::io::Read as _;
    let mut head = Vec::new();
    if let Ok(f) = std::fs::File::open(path) {
        let _ = f.take(16).read_to_end(&mut head);
    }
    if crate::image_input::sniff_media_type(&head).is_some() {
        return Dropped::Image(path.to_path_buf());
    }
    if head.starts_with(b"%PDF") {
        return Dropped::Pdf(path.to_path_buf());
    }
    let text = std::fs::read(path).ok().filter(|b| {
        b.len() <= crate::cli::commands::run::AT_FILE_MAX_BYTES && std::str::from_utf8(b).is_ok()
    });
    if text.is_some() {
        Dropped::Text(path.to_path_buf())
    } else {
        Dropped::Other(path.to_path_buf())
    }
}

/// `Some` only when the whole paste is file paths: each token must look like a path and either
/// be an existing file or an absent path from another machine. Anything else is ordinary text.
pub(crate) fn classify_paste(text: &str) -> Option<Vec<Dropped>> {
    let tokens = split_path_tokens(text);
    if tokens.is_empty() || !tokens.iter().all(|t| looks_like_path(t)) {
        return None;
    }
    let mut out = Vec::new();
    for t in &tokens {
        let path = normalize_path_arg(t);
        if path.is_file() {
            out.push(classify_file(&path));
        } else if !path.exists() && looks_foreign(&path) {
            out.push(Dropped::Foreign(path.display().to_string()));
        } else {
            return None;
        }
    }
    Some(out)
}

/// The `@mention` form that survives paths with spaces.
pub(crate) fn mention_for(path: &Path) -> String {
    let p = path.display().to_string();
    if p.chars().any(char::is_whitespace) {
        format!("@{{{p}}}")
    } else {
        format!("@{p}")
    }
}

/// Handle a paste / submitted line that is nothing but file paths. Returns whether it was
/// consumed (the caller must not also insert the text).
pub(crate) fn apply_paths(app: &mut forge_tui::App, text: &str) -> bool {
    let Some(items) = classify_paste(text) else {
        return false;
    };
    if items.iter().all(|i| matches!(i, Dropped::Foreign(_))) {
        note_foreign(app, &items);
        return false;
    }
    for item in items {
        match item {
            Dropped::Image(path) => {
                match crate::image_input::load_image_file(&path.to_string_lossy()) {
                    Ok((att, label)) => {
                        app.attach_image(att, &label);
                        app.note(&format!("📎 attached image ({label})"));
                    }
                    Err(e) => app.note(&format!("⚠ {e}")),
                }
            }
            Dropped::Text(path) => {
                app.handle_paste(format!("{} ", mention_for(&path)));
                app.note(&format!("📎 {} will be included", path.display()));
            }
            // TODO(pdf): attach as a document once a document attachment type exists.
            Dropped::Pdf(path) => app.note(&format!(
                "⚠ {}: PDF attachments are not supported yet",
                path.display()
            )),
            Dropped::Other(path) => app.note(&format!(
                "⚠ {}: not an image or text file, skipped",
                path.display()
            )),
            Dropped::Foreign(path) => app.note(&foreign_note(&path)),
        }
    }
    true
}

fn note_foreign(app: &mut forge_tui::App, items: &[Dropped]) {
    for item in items {
        if let Dropped::Foreign(path) = item {
            app.note(&foreign_note(path));
        }
    }
}

fn foreign_note(path: &str) -> String {
    format!(
        "⚠ {path} is not on this machine — copy it over (scp) or paste the image itself (Ctrl+V)"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png() -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::new(2, 2))
            .write_to(&mut buf, image::ImageFormat::Png)
            .unwrap();
        buf.into_inner()
    }

    #[test]
    fn unquotes_all_the_ways_a_terminal_hands_over_a_path() {
        assert_eq!(
            normalize_path_arg("'/tmp/a b.png'"),
            PathBuf::from("/tmp/a b.png")
        );
        assert_eq!(
            normalize_path_arg("\"/tmp/a b.png\""),
            PathBuf::from("/tmp/a b.png")
        );
        assert_eq!(
            normalize_path_arg("/tmp/a\\ b.png"),
            PathBuf::from("/tmp/a b.png")
        );
        assert_eq!(
            normalize_path_arg("file:///tmp/a%20b.png"),
            PathBuf::from("/tmp/a b.png")
        );
        assert_eq!(
            normalize_path_arg("file://localhost/tmp/x.png"),
            PathBuf::from("/tmp/x.png")
        );
        let home = std::env::var("HOME").unwrap();
        assert_eq!(
            normalize_path_arg("~/x.png"),
            PathBuf::from(home).join("x.png")
        );
    }

    #[test]
    fn splits_multiple_paths_with_quotes_and_escapes() {
        assert_eq!(
            split_path_tokens("/a/b.png '/c d/e.png' /f\\ g/h.png\n\"/i j/k.png\""),
            ["/a/b.png", "'/c d/e.png'", "/f\\ g/h.png", "\"/i j/k.png\""]
        );
    }

    #[test]
    fn paste_of_existing_files_is_classified_by_content_not_extension() {
        let dir = tempfile::tempdir().unwrap();
        let img = dir.path().join("shot.dat");
        std::fs::write(&img, png()).unwrap();
        let txt = dir.path().join("my notes.rs");
        std::fs::write(&txt, "fn main() {}").unwrap();
        let pdf = dir.path().join("a.pdf");
        std::fs::write(&pdf, b"%PDF-1.7 ...").unwrap();

        let paste = format!(
            "{} '{}' {}",
            img.display(),
            txt.display(),
            pdf.display().to_string().replace(' ', "\\ ")
        );
        let items = classify_paste(&paste).unwrap();
        assert_eq!(
            items,
            vec![Dropped::Image(img), Dropped::Text(txt), Dropped::Pdf(pdf)]
        );
    }

    #[test]
    fn file_uri_with_escapes_resolves() {
        let dir = tempfile::tempdir().unwrap();
        let img = dir.path().join("My Shot.png");
        std::fs::write(&img, png()).unwrap();
        let uri = format!("file://{}", img.display().to_string().replace(' ', "%20"));
        assert_eq!(classify_paste(&uri).unwrap(), vec![Dropped::Image(img)]);
    }

    #[test]
    fn ordinary_text_and_missing_local_paths_are_not_paths() {
        assert!(classify_paste("hello world").is_none());
        assert!(classify_paste("see /usr/bin/definitely-not-here-xyz").is_none());
        assert!(
            classify_paste("/tmp /tmp").is_none(),
            "directories are not attachable"
        );
        assert!(classify_paste("").is_none());
    }

    #[test]
    fn absent_laptop_path_is_foreign() {
        let p = "/home/someone-else-xyz/Pictures/shot.png";
        assert_eq!(
            classify_paste(p).unwrap(),
            vec![Dropped::Foreign(p.to_string())]
        );
        assert!(classify_paste("/Users/jane/Desktop/shot.png").is_some());
    }

    #[test]
    fn foreign_only_paste_notes_and_falls_through_to_text() {
        let mut app = forge_tui::App::default();
        assert!(!apply_paths(
            &mut app,
            "/home/someone-else-xyz/Pictures/shot.png"
        ));
    }

    #[test]
    fn apply_attaches_image_block_and_text_mention() {
        let dir = tempfile::tempdir().unwrap();
        let img = dir.path().join("s.png");
        std::fs::write(&img, png()).unwrap();
        let txt = dir.path().join("a b.txt");
        std::fs::write(&txt, "hi").unwrap();
        let mut app = forge_tui::App::default();
        assert!(apply_paths(
            &mut app,
            &format!("{} '{}'", img.display(), txt.display())
        ));
        assert!(app.input.contains("[image ("), "{}", app.input);
        assert!(
            app.input.contains(&format!("@{{{}}}", txt.display())),
            "{}",
            app.input
        );
        let (_, images) = app.resolve_paste_blocks(app.input.clone());
        assert_eq!(images.len(), 1);
    }
}
