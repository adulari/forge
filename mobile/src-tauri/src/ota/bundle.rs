//! Unpacking a downloaded bundle onto disk, and reading files back out of the live one.

use std::borrow::Cow;
use std::fs;
use std::io::{self, Cursor, Read};
use std::path::{Component, Path, PathBuf};

const MAX_TOTAL_BYTES: u64 = 512 * 1024 * 1024;
const MAX_FILE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_ENTRIES: usize = 20_000;
const TEMP_PREFIX: &str = ".unpack-";

/// Unpacks `zip` into `<bundles>/<id>` atomically: files are written to a sibling temp directory
/// that only becomes `<id>` by a rename once it is complete and holds an `index.html`.
pub fn unpack(zip: &[u8], bundles: &Path, id: &str) -> Result<PathBuf, String> {
    fs::create_dir_all(bundles).map_err(|error| format!("create bundles dir: {error}"))?;
    let temp = bundles.join(format!(
        "{TEMP_PREFIX}{id}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos())
    ));
    let result = write_all(zip, &temp).and_then(|()| {
        if !temp.join("index.html").is_file() {
            return Err("bundle has no index.html".to_string());
        }
        let target = bundles.join(id);
        if target.exists() {
            fs::remove_dir_all(&target).map_err(|error| format!("replace bundle: {error}"))?;
        }
        fs::rename(&temp, &target).map_err(|error| format!("publish bundle: {error}"))?;
        Ok(target)
    });
    if result.is_err() {
        let _ = fs::remove_dir_all(&temp);
    }
    result
}

fn write_all(zip: &[u8], temp: &Path) -> Result<(), String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(zip))
        .map_err(|error| format!("bundle is not a zip: {error}"))?;
    if archive.len() > MAX_ENTRIES {
        return Err("bundle has too many files".to_string());
    }
    fs::create_dir_all(temp).map_err(|error| format!("create temp dir: {error}"))?;
    let mut total = 0u64;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("read bundle entry: {error}"))?;
        let relative = entry
            .enclosed_name()
            .ok_or_else(|| format!("bundle entry escapes the bundle: {}", entry.name()))?;
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(format!("bundle entry is a symlink: {}", entry.name()));
        }
        let destination = temp.join(&relative);
        if entry.is_dir() {
            fs::create_dir_all(&destination).map_err(|error| format!("create dir: {error}"))?;
            continue;
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|error| format!("create dir: {error}"))?;
        }
        let mut file =
            fs::File::create(&destination).map_err(|error| format!("create file: {error}"))?;
        let copied = io::copy(&mut (&mut entry).take(MAX_FILE_BYTES + 1), &mut file)
            .map_err(|error| format!("unpack {}: {error}", entry.name()))?;
        total += copied;
        if copied > MAX_FILE_BYTES || total > MAX_TOTAL_BYTES {
            return Err("bundle expands beyond the size limit".to_string());
        }
    }
    Ok(())
}

/// Resolves an asset key such as `/_expo/static/js/app.js` to a file inside `root`, or `None` if
/// it would leave `root`.
pub fn resolve(root: &Path, key: &str) -> Option<PathBuf> {
    let relative = Path::new(key.trim_start_matches('/'));
    let mut path = root.to_path_buf();
    for component in relative.components() {
        match component {
            Component::Normal(part) => path.push(part),
            _ => return None,
        }
    }
    (path != root).then_some(path)
}

pub fn read(root: &Path, key: &str) -> Option<Cow<'static, [u8]>> {
    let path = resolve(root, key)?;
    fs::read(path).ok().map(Cow::Owned)
}

/// Removes bundle directories that nothing refers to, plus any interrupted unpack.
pub fn collect_garbage(bundles: &Path, keep: &[&str]) {
    let Ok(entries) = fs::read_dir(bundles) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !keep.contains(&name.as_str()) {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

#[cfg(test)]
pub mod test_support {
    use std::io::Write;

    pub fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        for (name, body) in files {
            writer.start_file(*name, options).unwrap();
            writer.write_all(body).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    pub fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("forge-ota-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;

    #[test]
    fn unpacks_atomically_into_the_bundle_directory() {
        let dir = scratch("unpack");
        let zip = zip_of(&[("index.html", b"<html>"), ("_expo/static/js/app.js", b"x")]);
        let target = unpack(&zip, &dir, "abcdef01").unwrap();
        assert_eq!(target, dir.join("abcdef01"));
        assert_eq!(fs::read(target.join("index.html")).unwrap(), b"<html>");
        assert_eq!(
            fs::read(target.join("_expo/static/js/app.js")).unwrap(),
            b"x"
        );
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(leftovers, vec!["abcdef01".to_string()]);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_failed_unpack_leaves_nothing_behind() {
        let dir = scratch("failed");
        assert!(unpack(&zip_of(&[("app.js", b"x")]), &dir, "abcdef01").is_err());
        assert!(unpack(b"not a zip", &dir, "abcdef01").is_err());
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_failed_unpack_keeps_the_existing_bundle() {
        let dir = scratch("keeps");
        unpack(&zip_of(&[("index.html", b"old")]), &dir, "abcdef01").unwrap();
        assert!(unpack(b"junk", &dir, "abcdef01").is_err());
        assert_eq!(fs::read(dir.join("abcdef01/index.html")).unwrap(), b"old");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn refuses_entries_that_escape_the_bundle() {
        let dir = scratch("slip");
        let zip = zip_of(&[("index.html", b"x"), ("../evil.txt", b"x")]);
        assert!(unpack(&zip, &dir, "abcdef01").is_err());
        assert!(!dir.join("evil.txt").exists());
        assert!(!dir.parent().unwrap().join("evil.txt").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn resolve_stays_inside_the_root() {
        let root = Path::new("/ota/bundle");
        assert_eq!(resolve(root, "/index.html"), Some(root.join("index.html")));
        assert_eq!(resolve(root, "/a/b.js"), Some(root.join("a/b.js")));
        assert_eq!(resolve(root, "/../secret"), None);
        assert_eq!(resolve(root, "/a/../../secret"), None);
        assert_eq!(resolve(root, "/"), None);
    }

    #[test]
    fn garbage_collection_keeps_only_referenced_bundles() {
        let dir = scratch("gc");
        for name in ["keep0001", "drop0002", ".unpack-half"] {
            fs::create_dir_all(dir.join(name)).unwrap();
        }
        collect_garbage(&dir, &["keep0001"]);
        assert!(dir.join("keep0001").exists());
        assert!(!dir.join("drop0002").exists());
        assert!(!dir.join(".unpack-half").exists());
        fs::remove_dir_all(dir).unwrap();
    }
}
