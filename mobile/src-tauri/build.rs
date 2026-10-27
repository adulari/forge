// The runtime version and embedded-bundle identity are shared with CI through
// `runtime_version.rs`; see docs/features/desktop-ota.md.
#[path = "runtime_version.rs"]
mod runtime_version;

use std::path::Path;
use std::process::Command;

fn vcs(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?.trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn main() {
    // src/vblank.rs: libwebkit's `drmWaitVBlank` calls only resolve to the executable's definition
    // when the executable exports it in its dynamic symbol table.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        println!("cargo:rustc-link-arg-bins=-Wl,--export-dynamic-symbol=drmWaitVBlank");
    }

    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for file in runtime_version::tracked_files(root).unwrap_or_default() {
        println!("cargo:rerun-if-changed={}", file.display());
    }
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=capabilities");
    println!("cargo:rerun-if-env-changed=FORGE_EMBEDDED_BUNDLE_ID");
    println!("cargo:rerun-if-env-changed=FORGE_EMBEDDED_CREATED_AT");
    if let Some(head) = vcs(&["rev-parse", "--git-path", "HEAD"]) {
        println!("cargo:rerun-if-changed={head}");
    }

    let runtime = runtime_version::compute(root).expect("compute the desktop runtime version");
    println!("cargo:rustc-env=FORGE_DESKTOP_RUNTIME_VERSION={runtime}");

    // The embedded UI's identity, so an OTA bundle is only taken when it is strictly newer.
    // Commit time (not build time) keeps this comparable with the bundle CI publishes for a commit.
    let bundle_id = std::env::var("FORGE_EMBEDDED_BUNDLE_ID")
        .ok()
        .or_else(|| vcs(&["rev-parse", "HEAD"]))
        .unwrap_or_else(|| "unknown".to_string());
    let created_at = std::env::var("FORGE_EMBEDDED_CREATED_AT")
        .ok()
        .or_else(|| vcs(&["show", "-s", "--format=%ct", "HEAD"]))
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    println!("cargo:rustc-env=FORGE_EMBEDDED_BUNDLE_ID={bundle_id}");
    println!("cargo:rustc-env=FORGE_EMBEDDED_CREATED_AT={created_at}");

    tauri_build::build()
}
