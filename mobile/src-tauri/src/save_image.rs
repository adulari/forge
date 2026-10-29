//! Saving an image from the viewer. The webview cannot start a download (the shell registers no
//! download handler), so the page sends the bytes here and they land in the user's Downloads
//! folder under a name that never overwrites an existing file.

use std::path::{Path, PathBuf};

use tauri::ipc::{InvokeBody, Request};

const MAX_BYTES: usize = 40 * 1024 * 1024;
const EXTENSIONS: [&str; 5] = ["png", "jpg", "gif", "webp", "svg"];

fn percent_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let pair = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(value) = pair.and_then(|hex| u8::from_str_radix(hex, 16).ok()) {
                out.push(value);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn is_image(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        || bytes.starts_with(&[0xFF, 0xD8, 0xFF])
        || bytes.starts_with(b"GIF8")
        || (bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP")
        || String::from_utf8_lossy(&bytes[..bytes.len().min(512)]).contains("<svg")
}

/// A file name that stays inside one folder: the last path component, odd characters replaced,
/// and an image extension.
pub(crate) fn safe_name(raw: &str) -> String {
    let last = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = last
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | ' ') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches(|c| c == '.' || c == ' ');
    let cleaned: String = cleaned.chars().take(80).collect();
    let stem = if cleaned.is_empty() {
        "image".to_string()
    } else {
        cleaned
    };
    let has_extension = Path::new(&stem)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()) || e.eq_ignore_ascii_case("jpeg")
        })
        .unwrap_or(false);
    if has_extension {
        stem
    } else {
        format!("{stem}.png")
    }
}

/// `dir/name`, or `dir/stem (2).ext` and so on when that file already exists.
pub(crate) fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let path = Path::new(name);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("image");
    let extension = path.extension().and_then(|s| s.to_str()).unwrap_or("png");
    (2..)
        .map(|n| dir.join(format!("{stem} ({n}).{extension}")))
        .find(|candidate| !candidate.exists())
        .unwrap_or(first)
}

fn downloads_dir() -> Result<PathBuf, String> {
    let user = directories::UserDirs::new().ok_or("no home folder on this machine")?;
    let dir = user
        .download_dir()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| user.home_dir().join("Downloads"));
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    Ok(dir)
}

pub(crate) fn save_into(dir: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    if bytes.is_empty() || bytes.len() > MAX_BYTES {
        return Err("the image is empty or too large to save".into());
    }
    if !is_image(bytes) {
        return Err("that is not an image".into());
    }
    let path = unique_path(dir, &safe_name(name));
    std::fs::write(&path, bytes).map_err(|e| format!("could not write {}: {e}", path.display()))?;
    Ok(path)
}

/// The image bytes are the raw request body; `x-file-name` carries the suggested name.
#[tauri::command]
pub fn save_image(request: Request<'_>) -> Result<String, String> {
    let InvokeBody::Raw(bytes) = request.body() else {
        return Err("expected the raw image bytes".into());
    };
    let name = request
        .headers()
        .get("x-file-name")
        .and_then(|value| value.to_str().ok())
        .map(percent_decode)
        .unwrap_or_default();
    let path = save_into(&downloads_dir()?, &name, bytes)?;
    Ok(path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nrest";

    fn scratch(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("forge-save-image-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn names_stay_inside_the_folder_and_keep_an_image_extension() {
        assert_eq!(safe_name("shot.PNG"), "shot.PNG");
        assert_eq!(safe_name("../../etc/passwd"), "passwd.png");
        assert_eq!(safe_name("a:b*c"), "a_b_c.png");
        assert_eq!(safe_name(""), "image.png");
        assert_eq!(safe_name("..."), "image.png");
        assert_eq!(safe_name("photo.jpeg"), "photo.jpeg");
    }

    #[test]
    fn percent_escapes_decode_and_bad_ones_pass_through() {
        assert_eq!(percent_decode("cart%20page.png"), "cart page.png");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
        assert_eq!(percent_decode("%C3%A9"), "é");
    }

    #[test]
    fn a_name_that_exists_gets_a_counter() {
        let dir = scratch("counter");
        let first = save_into(&dir, "shot.png", PNG).unwrap();
        let second = save_into(&dir, "shot.png", PNG).unwrap();
        assert_eq!(first.file_name().unwrap(), "shot.png");
        assert_eq!(second.file_name().unwrap(), "shot (2).png");
    }

    #[test]
    fn only_images_are_written() {
        let dir = scratch("only-images");
        assert!(save_into(&dir, "x.png", b"not an image").is_err());
        assert!(save_into(&dir, "x.png", b"").is_err());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
    }
}
