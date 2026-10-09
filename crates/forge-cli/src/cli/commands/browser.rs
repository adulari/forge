use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use forge_browser::attach::{
    attach_launch_args, default_attach_profile, discover, find_browsers, normalize_endpoint,
};

use crate::cli::args::BrowserOp;

fn data_home() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from(".forge"))
}

/// `forge browser attach`: start a browser Forge's `browser` tool can attach to.
pub(crate) async fn browser_cmd(op: BrowserOp) -> Result<()> {
    let BrowserOp::Attach {
        port,
        profile,
        browser,
        print,
    } = op;
    let url = normalize_endpoint(&port.to_string())?;
    let profile = profile.unwrap_or_else(|| default_attach_profile(&data_home()));

    if let Ok(found) = discover(&url).await {
        println!(
            "a browser ({}) is already listening at {url}",
            found.browser
        );
        print_config(&url);
        return Ok(());
    }

    let binary = match browser {
        Some(path) => path,
        None => find_browsers().into_iter().next().map(|b| b.path).context(
            "no Chromium-family browser found (looked for Chrome, Chromium, Brave, Edge); \
                 pass --browser <path>",
        )?,
    };
    let args = attach_launch_args(&profile, port);
    if print {
        println!("{} {}", binary.display(), args.join(" "));
        print_config(&url);
        return Ok(());
    }
    std::fs::create_dir_all(&profile)
        .with_context(|| format!("create profile directory {}", profile.display()))?;
    let mut command = std::process::Command::new(&binary);
    command
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    command
        .spawn()
        .with_context(|| format!("launch {}", binary.display()))?;

    for _ in 0..50 {
        if discover(&url).await.is_ok() {
            println!("browser started with profile {}", profile.display());
            println!(
                "log in to the sites you want Forge to use in that window (once; it persists)."
            );
            print_config(&url);
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    bail!(
        "{} started but nothing answers at {url}/json/version. If this browser is already \
         running on another profile, close it first — a second launch hands off to the first and \
         never opens the debugging port",
        binary.display()
    )
}

fn print_config(url: &str) {
    println!("attach Forge with either:");
    println!("  [browser]\n  attach = \"{url}\"");
    println!("  FORGE_BROWSER_CDP={url}");
    println!(
        "this profile holds real logged-in sessions; anything that can reach {url} can drive them."
    );
}
