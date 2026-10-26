//! CI companion of the desktop app's OTA code.
//!
//!   runtime-version [SRC_TAURI]                  print the runtime version for that checkout
//!   runtime-version verify SRC_TAURI FILE SIG    check a signature against the updater public
//!                                                key in tauri.conf.json, with the app's own code
#![allow(dead_code)]

#[path = "../../../runtime_version.rs"]
mod runtime_version;

#[path = "../../../src/ota/manifest.rs"]
mod manifest;

use std::path::Path;

fn fail(message: &str) -> ! {
    eprintln!("runtime-version: {message}");
    std::process::exit(1);
}

fn verify(src_tauri: &str, file: &str, signature: &str) {
    let config = std::fs::read_to_string(Path::new(src_tauri).join("tauri.conf.json"))
        .unwrap_or_else(|error| fail(&format!("read tauri.conf.json: {error}")));
    let config: serde_json::Value =
        serde_json::from_str(&config).unwrap_or_else(|error| fail(&format!("parse config: {error}")));
    let pubkey = config["plugins"]["updater"]["pubkey"]
        .as_str()
        .unwrap_or_else(|| fail("tauri.conf.json has no updater pubkey"));
    let data = std::fs::read(file).unwrap_or_else(|error| fail(&format!("read {file}: {error}")));
    let signature = std::fs::read_to_string(signature)
        .unwrap_or_else(|error| fail(&format!("read {signature}: {error}")));
    if let Err(error) = manifest::verify_signature(pubkey, &data, &signature) {
        fail(&error);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("verify") {
        let [_, src_tauri, file, signature] = args.as_slice() else {
            fail("usage: runtime-version verify SRC_TAURI FILE SIG");
        };
        verify(src_tauri, file, signature);
        return;
    }
    let dir = args.first().map_or(".", String::as_str);
    match runtime_version::compute(Path::new(dir)) {
        Ok(version) => println!("{version}"),
        Err(error) => fail(&error.to_string()),
    }
}
