//! The desktop runtime version: a fingerprint of everything native that the web UI depends on.
//!
//! Shared by `build.rs` (bakes it into the binary) and `tools/runtime-version` (what CI stamps
//! on a published bundle), so both sides hash the same inputs the same way. A UI bundle is only
//! applied to a binary whose runtime version equals the one it was published for.

use sha2::{Digest, Sha256};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Config keys that describe the native shell. `version`, `build` and `bundle` are left out on
/// purpose: release CI stamps `version` into tauri.conf.json and none of the three change what
/// the web UI can call.
const CONFIG_KEYS: [&str; 4] = ["identifier", "productName", "app", "plugins"];

pub fn compute(src_tauri: &Path) -> io::Result<String> {
    let mut hasher = Sha256::new();
    for path in tracked_files(src_tauri)? {
        let relative = path
            .strip_prefix(src_tauri)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let body = if relative == "tauri.conf.json" {
            canonical_config(&fs::read_to_string(&path)?)?.into_bytes()
        } else {
            normalize_newlines(&fs::read(&path)?)
        };
        feed(&mut hasher, &relative, &body);
    }
    let digest = hasher.finalize();
    Ok(digest
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn feed(hasher: &mut Sha256, name: &str, body: &[u8]) {
    hasher.update(name.len().to_le_bytes());
    hasher.update(name.as_bytes());
    hasher.update(body.len().to_le_bytes());
    hasher.update(body);
}

/// Every file whose change can alter what the binary exposes to the web UI.
pub fn tracked_files(src_tauri: &Path) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for name in [
        "build.rs",
        "runtime_version.rs",
        "Cargo.toml",
        "Cargo.lock",
        "tauri.conf.json",
        "Info.plist",
    ] {
        let path = src_tauri.join(name);
        if path.is_file() {
            files.push(path);
        }
    }
    collect(&src_tauri.join("src"), "rs", &mut files)?;
    collect(&src_tauri.join("capabilities"), "json", &mut files)?;
    files.sort();
    Ok(files)
}

fn collect(dir: &Path, extension: &str, out: &mut Vec<PathBuf>) -> io::Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect(&path, extension, out)?;
        } else if path.extension().and_then(|value| value.to_str()) == Some(extension) {
            out.push(path);
        }
    }
    Ok(())
}

fn normalize_newlines(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\r' && bytes.get(index + 1) == Some(&b'\n') {
            index += 1;
            continue;
        }
        out.push(bytes[index]);
        index += 1;
    }
    out
}

fn canonical_config(raw: &str) -> io::Result<String> {
    let value: serde_json::Value = serde_json::from_str(raw)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let mut out = String::new();
    out.push('{');
    for (index, key) in CONFIG_KEYS.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(&format!("{key:?}:"));
        write_canonical(
            value.get(*key).unwrap_or(&serde_json::Value::Null),
            &mut out,
        );
    }
    out.push('}');
    Ok(out)
}

// serde_json's Map ordering depends on whether some other crate in the build graph turned on
// `preserve_order`, so key order is sorted by hand to keep build.rs and the CI tool in agreement.
fn write_canonical(value: &serde_json::Value, out: &mut String) {
    match value {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&format!("{key:?}:"));
                write_canonical(&map[key], out);
            }
            out.push('}');
        }
        serde_json::Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "forge-runtime-version-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::create_dir_all(dir.join("capabilities")).unwrap();
        fs::write(dir.join("src/lib.rs"), "fn main() {}\n").unwrap();
        fs::write(dir.join("Cargo.lock"), "lock\n").unwrap();
        fs::write(dir.join("capabilities/default.json"), "{}\n").unwrap();
        fs::write(
            dir.join("tauri.conf.json"),
            r#"{"version":"1.0.0","identifier":"a.b","plugins":{"x":{"k":1,"a":2}},"build":{"frontendDist":"../dist"}}"#,
        )
        .unwrap();
        dir
    }

    #[test]
    fn stable_across_version_stamp_key_order_and_newlines() {
        let dir = fixture("stable");
        let before = compute(&dir).unwrap();
        fs::write(
            dir.join("tauri.conf.json"),
            "{\r\n\"build\":{\"frontendDist\":\"other\"},\"plugins\":{\"x\":{\"a\":2,\"k\":1}},\"identifier\":\"a.b\",\"version\":\"9.9.9\"}",
        )
        .unwrap();
        fs::write(dir.join("src/lib.rs"), "fn main() {}\r\n").unwrap();
        assert_eq!(compute(&dir).unwrap(), before);
        assert_eq!(before.len(), 16);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn matches_the_value_build_rs_baked_into_this_binary() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        assert_eq!(
            compute(root).unwrap(),
            env!("FORGE_DESKTOP_RUNTIME_VERSION")
        );
    }

    #[test]
    fn changes_with_every_native_input() {
        let dir = fixture("native");
        let base = compute(&dir).unwrap();
        let mutations: [(&str, &str); 4] = [
            ("src/lib.rs", "fn main() { tauri_command(); }\n"),
            ("Cargo.lock", "other lock\n"),
            ("capabilities/default.json", "{\"permissions\":[]}\n"),
            (
                "tauri.conf.json",
                r#"{"identifier":"a.b","plugins":{"x":{"k":2}}}"#,
            ),
        ];
        for (file, content) in mutations {
            let original = fs::read(dir.join(file)).unwrap();
            fs::write(dir.join(file), content).unwrap();
            assert_ne!(
                compute(&dir).unwrap(),
                base,
                "{file} must change the runtime version"
            );
            fs::write(dir.join(file), original).unwrap();
        }
        assert_eq!(compute(&dir).unwrap(), base);
        fs::remove_dir_all(dir).unwrap();
    }
}
