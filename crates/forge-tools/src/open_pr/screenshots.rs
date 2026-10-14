//! Which ledger screenshots may leave the machine.
//!
//! The manifest is a plain file in the workspace, so anything that can write there can name any
//! path in a `file` field. Before a file is pushed to a branch other people can read, it must be a
//! plain image name inside the session's evidence directory, a regular file (not a symlink), and
//! byte-for-byte the capture the manifest hashed.

use std::path::PathBuf;

use crate::evidence::{sha256_hex, EvidenceKind, Ledger};

const IMAGE_EXTENSIONS: [&str; 5] = ["png", "jpg", "jpeg", "webp", "gif"];

pub(super) struct Selection {
    /// (evidence file name, local path) for each screenshot safe to publish.
    pub files: Vec<(String, PathBuf)>,
    /// Manifest entries that were refused.
    pub refused: Vec<String>,
}

pub(super) fn publishable(ledger: &Ledger) -> Selection {
    let mut selection = Selection {
        files: Vec::new(),
        refused: Vec::new(),
    };
    for item in ledger.items() {
        if item.kind != EvidenceKind::Screenshot {
            continue;
        }
        if !plain_image_name(&item.file) {
            selection.refused.push(item.file);
            continue;
        }
        let path = ledger.dir().join(&item.file);
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        let intact = meta.file_type().is_file()
            && std::fs::read(&path).is_ok_and(|bytes| sha256_hex(&bytes) == item.sha256);
        if intact {
            selection.files.push((item.file, path));
        } else {
            selection.refused.push(item.file);
        }
    }
    selection
}

fn plain_image_name(name: &str) -> bool {
    let Some((stem, extension)) = name.rsplit_once('.') else {
        return false;
    };
    !stem.is_empty()
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && !name.contains("..")
        && IMAGE_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence::Capture;
    use std::io::Write as _;

    #[test]
    fn only_intact_images_inside_the_evidence_dir_are_publishable() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = Ledger::new(dir.path(), "sess-s").unwrap();
        let (_, good) = ledger
            .record(
                EvidenceKind::Screenshot,
                "browser",
                "png",
                b"png",
                Capture::default(),
            )
            .unwrap();
        let (tampered, _) = ledger
            .record(
                EvidenceKind::Screenshot,
                "browser",
                "png",
                b"original",
                Capture::default(),
            )
            .unwrap();
        std::fs::write(&tampered, b"swapped").unwrap();
        let secret = dir.path().join("id_ed25519");
        std::fs::write(&secret, b"key").unwrap();
        let sha = sha256_hex(b"key");
        let mut manifest = std::fs::OpenOptions::new()
            .append(true)
            .open(ledger.dir().join(crate::evidence::MANIFEST))
            .unwrap();
        for file in ["../../../id_ed25519", "0009-link.png"] {
            writeln!(
                manifest,
                r#"{{"ts":"t","kind":"screenshot","file":"{file}","sha256":"{sha}","source":"browser"}}"#
            )
            .unwrap();
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(&secret, ledger.dir().join("0009-link.png")).unwrap();

        let selection = publishable(&ledger);
        let names: Vec<&str> = selection.files.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec![good.file.as_str()]);
        assert_eq!(selection.refused.len(), 3, "{:?}", selection.refused);
    }

    #[test]
    fn image_names_are_plain() {
        assert!(plain_image_name("0001-browser.png"));
        assert!(plain_image_name("0002-device.JPG"));
        for name in [
            "../x.png",
            "a/b.png",
            ".png",
            "x.txt",
            "x",
            "a..png",
            ".hidden.png",
        ] {
            assert!(!plain_image_name(name), "{name}");
        }
    }
}
