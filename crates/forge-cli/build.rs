//! Stamp the build with the git revision for issue reports (`FORGE_GIT_SHA`). Best effort: a
//! crates.io build has no git checkout, and reports then say `unknown`.

fn main() {
    println!("cargo:rerun-if-env-changed=FORGE_GIT_SHA");
    println!("cargo:rerun-if-env-changed=GITHUB_SHA");
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    let sha = std::env::var("FORGE_GIT_SHA")
        .ok()
        .or_else(|| std::env::var("GITHUB_SHA").ok())
        .or_else(|| {
            let out = std::process::Command::new("git")
                .args(["rev-parse", "--short=12", "HEAD"])
                .output()
                .ok()?;
            out.status
                .success()
                .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
        })
        .map(|s| {
            s.chars()
                .filter(char::is_ascii_hexdigit)
                .take(12)
                .collect::<String>()
        })
        .filter(|s| s.len() >= 7);
    if let Some(sha) = sha {
        println!("cargo:rustc-env=FORGE_GIT_SHA={sha}");
    }
}
