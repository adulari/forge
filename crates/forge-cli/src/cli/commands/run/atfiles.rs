/// Enumerate project files for `@path` completion: `git ls-files` first, then a portable directory
/// walk. The fallback used to shell out to Unix `find`, which silently produced nothing on Windows
/// (where `find.exe` is an unrelated text-search tool) — so `@path` completion was dead outside a git
/// repo on Windows. A plain `std::fs` walk works everywhere and needs no external program.
pub(crate) fn load_at_files() -> Vec<String> {
    if let Ok(out) = std::process::Command::new("git")
        .args(["ls-files", "--cached", "--others", "--exclude-standard"])
        .output()
    {
        if out.status.success() {
            return String::from_utf8_lossy(&out.stdout)
                .lines()
                .map(|s| s.to_string())
                .collect();
        }
    }
    let base = std::path::Path::new(".");
    let mut out = Vec::new();
    walk_at_files(base, base, 5, &mut out);
    out
}

/// Paths git reports as changed (staged, unstaged or untracked), for the picker's `modified` mark.
/// Empty outside a repository; one cheap `git status` per picker open.
fn load_modified_files() -> Vec<String> {
    let Ok(out) = std::process::Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=all"])
        .output()
    else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.len() > 3)
        .map(|l| {
            let path = &l[3..];
            path.rsplit(" -> ")
                .next()
                .unwrap_or(path)
                .trim_matches('"')
                .to_string()
        })
        .collect()
}

/// Recursive file walk for [`load_at_files`]: up to `depth` levels under `base`, files only,
/// skipping dot-entries, with `/`-normalized paths relative to `base`. Bounded so a giant tree can't
/// stall completion.
fn walk_at_files(
    dir: &std::path::Path,
    base: &std::path::Path,
    depth: usize,
    out: &mut Vec<String>,
) {
    if depth == 0 || out.len() >= 10_000 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue; // skip .git, dotfiles, hidden dirs
        }
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            walk_at_files(&path, base, depth - 1, out);
        } else if ft.is_file() {
            let rel = path.strip_prefix(base).unwrap_or(&path);
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
        if out.len() >= 10_000 {
            return;
        }
    }
}

/// Keep the `@path` picker in sync with the `@token` at the cursor: open + filter when present,
/// close when the token disappears. Files are loaded once on first open (cache lives in picker).
pub(crate) fn sync_at_picker_to_at_token(app: &mut forge_tui::App) {
    let cur = app.input_cursor.min(app.input.len());
    if let Some(tok) = forge_tui::at_token_at(&app.input, cur) {
        if app.at_picker.open {
            app.at_picker.query = tok.query;
            app.at_picker.clamp();
        } else {
            let mut files = crate::mentions::picker_entries();
            files.extend(load_at_files());
            app.at_picker.open_with(&tok.query, files);
            app.at_picker.set_modified(load_modified_files());
        }
    } else {
        app.at_picker.close();
    }
}

/// Cap on a single `@file`'s injected size, so dropping a huge file into context can't blow the
/// window. Larger files are skipped with a note rather than truncated mid-token.
pub(crate) const AT_FILE_MAX_BYTES: usize = 96 * 1024;

/// Read the `@path` file references in a submitted prompt and return them as guidance context
/// blocks (one per file) plus the list of paths actually included. The `@path` token stays in the
/// user's text (echoed verbatim); the contents ride along as separate guidance so the displayed
/// line stays clean. Missing paths are treated as ordinary text (silently skipped — `@` is also a
/// mention sigil); binary/oversized files are skipped with a visible note.
pub(crate) fn expand_at_files(prompt: &str) -> (Vec<String>, Vec<String>, Vec<String>) {
    expand_at_files_impl(prompt, None)
}

/// Remote/mobile counterpart to [`expand_at_files`]. Relative paths resolve against the
/// addressed session's canonical workspace and cannot escape it through `..`, absolute paths,
/// or symlinks. The unscoped wrapper above remains for the local TUI, whose process cwd is itself
/// the user's chosen workspace.
pub(crate) fn expand_at_files_in(
    prompt: &str,
    workspace: &std::path::Path,
) -> (Vec<String>, Vec<String>, Vec<String>) {
    expand_at_files_impl(prompt, Some(workspace))
}

fn at_paths(prompt: &str) -> Vec<String> {
    let chars: Vec<(usize, char)> = prompt.char_indices().collect();
    let mut paths = Vec::new();
    let mut index = 0usize;
    while index < chars.len() {
        let (byte, character) = chars[index];
        let boundary = index == 0 || chars[index - 1].1.is_whitespace();
        if character != '@' || !boundary {
            index += 1;
            continue;
        }
        let value_start = byte + character.len_utf8();
        if chars.get(index + 1).is_some_and(|(_, next)| *next == '{') {
            let content_start = chars[index + 1].0 + 1;
            let mut end_index = index + 2;
            while end_index < chars.len() && chars[end_index].1 != '}' {
                end_index += 1;
            }
            if end_index < chars.len() {
                let path = &prompt[content_start..chars[end_index].0];
                if !path.is_empty() {
                    paths.push(path.to_string());
                }
                index = end_index + 1;
                continue;
            }
        }

        let mut end_index = index + 1;
        while end_index < chars.len() && !chars[end_index].1.is_whitespace() {
            end_index += 1;
        }
        let end_byte = chars
            .get(end_index)
            .map(|(position, _)| *position)
            .unwrap_or(prompt.len());
        let path = &prompt[value_start..end_byte];
        if !path.is_empty() {
            paths.push(path.to_string());
        }
        index = end_index;
    }
    paths
}

fn scoped_at_path(workspace: &std::path::Path, raw: &str) -> Option<std::path::PathBuf> {
    let requested = std::path::Path::new(raw);
    let root = std::fs::canonicalize(workspace).ok()?;
    let candidate = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        root.join(requested)
    };
    let path = std::fs::canonicalize(candidate).ok()?;
    let relative = path.strip_prefix(&root).ok()?;
    for component in relative.components() {
        match component {
            std::path::Component::Normal(part) if part != ".git" => {}
            std::path::Component::CurDir => {}
            _ => return None,
        }
    }
    Some(path)
}

/// Resolve each distinct `@path` in `prompt` to a real path, applying workspace confinement when
/// one is given. Paths that do not resolve are left to be plain text.
fn resolved_at_paths(
    prompt: &str,
    workspace: Option<&std::path::Path>,
) -> Vec<(String, std::path::PathBuf)> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    // The scanner is UTF-8-safe and accepts `@{path with spaces}` in addition to the original
    // whitespace-delimited `@path` syntax. The braced form is what graphical file pickers insert
    // for a legitimate workspace path containing spaces.
    for path in at_paths(prompt) {
        if !seen.insert(path.clone()) {
            continue;
        }
        let resolved = workspace
            .and_then(|root| scoped_at_path(root, &path))
            .or_else(|| workspace.is_none().then(|| std::path::PathBuf::from(&path)));
        if let Some(resolved) = resolved {
            out.push((path, resolved));
        }
    }
    out
}

fn is_image_file(path: &std::path::Path) -> bool {
    use std::io::Read as _;
    let mut head = Vec::new();
    if let Ok(file) = std::fs::File::open(path) {
        let _ = file.take(16).read_to_end(&mut head);
    }
    crate::image_input::sniff(&head).is_some()
}

/// Attach every `@path` that is an image file (by content) as vision input for the next turn.
/// `workspace` confines remote prompts exactly like [`expand_at_files_in`]. An image already
/// pending on the session (e.g. a remote upload that also rides as a mention) is not added twice.
pub(crate) async fn attach_at_images(
    prompt: &str,
    workspace: Option<&std::path::Path>,
    session: &std::sync::Arc<tokio::sync::Mutex<forge_core::Session>>,
    app: &mut forge_tui::App,
) {
    for (_, resolved) in resolved_at_paths(prompt, workspace) {
        if !is_image_file(&resolved) {
            continue;
        }
        match crate::image_input::load_image_file(&resolved.to_string_lossy()) {
            Ok((att, label)) => {
                let mut s = session.lock().await;
                let mut pending = s.take_pending_images();
                if !pending.contains(&att) {
                    pending.push(att);
                    app.note(&format!("🖼 image attached ({label})"));
                }
                s.attach_images(pending);
            }
            Err(e) => app.note(&format!("⚠ {e}")),
        }
    }
}

/// Headless counterpart of [`attach_at_images`]: the image files named by `@path` mentions, loaded
/// for `Session::attach_images`. Failures are reported on stderr, never dropped silently.
pub(crate) fn at_image_attachments(prompt: &str) -> Vec<forge_types::ImageAttachment> {
    let mut out = Vec::new();
    for (_, resolved) in resolved_at_paths(prompt, None) {
        if !is_image_file(&resolved) {
            continue;
        }
        match crate::image_input::load_image_file(&resolved.to_string_lossy()) {
            Ok((att, _)) if !out.contains(&att) => out.push(att),
            Ok(_) => {}
            Err(e) => eprintln!("skipped image mention: {e}"),
        }
    }
    out
}

fn expand_at_files_impl(
    prompt: &str,
    workspace: Option<&std::path::Path>,
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let (mut blocks, mut included, mut skipped) = (Vec::new(), Vec::new(), Vec::new());
    for (path, resolved) in resolved_at_paths(prompt, workspace) {
        match std::fs::read(resolved) {
            // Images ride as vision input ([`attach_at_images`]), not as file text.
            Ok(raw) if crate::image_input::sniff(&raw).is_some() => {}
            Ok(raw) if raw.len() > AT_FILE_MAX_BYTES => {
                skipped.push(format!("@{path} (>{}KB)", AT_FILE_MAX_BYTES / 1024));
            }
            Ok(raw) => match String::from_utf8(raw) {
                Ok(text) => {
                    blocks.push(format!("Referenced file `{path}`:\n```\n{text}\n```"));
                    included.push(path.to_string());
                }
                Err(_) => skipped.push(format!("@{path} (binary)")),
            },
            Err(_) => {} // not a real file — leave as plain text
        }
    }
    (blocks, included, skipped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn braced_mentions_support_workspace_paths_with_spaces() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("My Docs")).unwrap();
        std::fs::write(directory.path().join("My Docs/plan.md"), "the plan").unwrap();

        let (blocks, included, skipped) =
            expand_at_files_in("review @{My Docs/plan.md}", directory.path());
        assert_eq!(included, vec!["My Docs/plan.md"]);
        assert!(blocks[0].contains("the plan"));
        assert!(skipped.is_empty());
    }

    #[test]
    fn image_mentions_are_not_binary_skips_and_attach_as_vision_input() {
        let directory = tempfile::tempdir().unwrap();
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::new(2, 2))
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        std::fs::write(directory.path().join("shot.png"), png.into_inner()).unwrap();
        std::fs::write(
            directory.path().join("blob.bin"),
            [0xffu8, 0xfe, 0x00, 0x80],
        )
        .unwrap();

        let prompt = "see @shot.png and @blob.bin";
        let (blocks, included, skipped) = expand_at_files_in(prompt, directory.path());
        assert!(blocks.is_empty() && included.is_empty());
        assert_eq!(skipped, vec!["@blob.bin (binary)"]);

        let images: Vec<_> = resolved_at_paths(prompt, Some(directory.path()))
            .into_iter()
            .filter(|(_, p)| is_image_file(p))
            .map(|(n, _)| n)
            .collect();
        assert_eq!(images, vec!["shot.png"]);
    }

    #[tokio::test]
    async fn attach_at_images_loads_once_even_if_already_pending() {
        use forge_core::Session;
        let directory = tempfile::tempdir().unwrap();
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::new(2, 2))
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let file = directory.path().join("shot.png");
        std::fs::write(&file, png.into_inner()).unwrap();
        let config = forge_config::Config::default();
        let session = std::sync::Arc::new(tokio::sync::Mutex::new(
            Session::start(
                std::sync::Arc::new(forge_store::Store::open_in_memory().unwrap()),
                std::sync::Arc::new(forge_provider::MockProvider),
                std::sync::Arc::new(forge_mesh::HeuristicRouter::new(config.clone())),
                forge_tools::ToolRegistry::with_core_tools(),
                Box::new(forge_tui::HeadlessPresenter::new(false)),
                config,
                ".",
            )
            .unwrap(),
        ));
        let mut app = forge_tui::App::default();
        let prompt = "@shot.png";
        attach_at_images(prompt, Some(directory.path()), &session, &mut app).await;
        attach_at_images(prompt, Some(directory.path()), &session, &mut app).await;
        assert_eq!(session.lock().await.take_pending_images().len(), 1);
    }

    #[test]
    fn headless_image_mentions_become_attachments() {
        let directory = tempfile::tempdir().unwrap();
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::new(2, 2))
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let file = directory.path().join("shot.png");
        std::fs::write(&file, png.into_inner()).unwrap();
        let prompt = format!("look at @{} and @{}", file.display(), file.display());
        assert_eq!(at_image_attachments(&prompt).len(), 1, "deduplicated");
        assert!(at_image_attachments("no mentions here").is_empty());
    }

    #[test]
    fn remote_mentions_cannot_escape_the_session_workspace() {
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), "nope").unwrap();
        let absolute = outside.path().join("secret");
        let prompt = format!("@../secret @{}", absolute.display());

        let (blocks, included, skipped) = expand_at_files_in(&prompt, workspace.path());
        assert!(blocks.is_empty());
        assert!(included.is_empty());
        assert!(skipped.is_empty());
    }
}
