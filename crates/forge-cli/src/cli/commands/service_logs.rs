//! `forge service logs`: the daemon's recent output, without knowing where the OS keeps it.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};

use super::service::SYSTEMD_UNIT_NAME;

/// Where a launchd agent's output goes. launchd has no journal, so the plist points both streams
/// at this file.
pub(crate) fn launchd_log_path() -> Option<PathBuf> {
    forge_config::data_dir().map(|dir| dir.join("serve.log"))
}

#[derive(Debug, PartialEq, Eq)]
struct Invocation {
    program: &'static str,
    args: Vec<String>,
}

fn invocation(
    os: &str,
    lines: u32,
    follow: bool,
    launchd_log: Option<&PathBuf>,
) -> Result<Invocation> {
    match os {
        "linux" => {
            let mut args = vec![
                "--user".to_string(),
                "-u".to_string(),
                SYSTEMD_UNIT_NAME.to_string(),
                "-n".to_string(),
                lines.to_string(),
                "--no-pager".to_string(),
            ];
            if follow {
                args.push("-f".to_string());
            }
            Ok(Invocation {
                program: "journalctl",
                args,
            })
        }
        "macos" => {
            let path = launchd_log
                .context("cannot resolve the data directory the launchd log lives in")?;
            let mut args = vec!["-n".to_string(), lines.to_string()];
            if follow {
                args.push("-f".to_string());
            }
            args.push(path.display().to_string());
            Ok(Invocation {
                program: "tail",
                args,
            })
        }
        _ => bail!(
            "forge service keeps no log on this platform; run `forge serve` in a terminal to see \
             its output"
        ),
    }
}

pub(crate) fn logs_cmd(lines: u32, follow: bool) -> Result<()> {
    let log = launchd_log_path();
    let plan = invocation(std::env::consts::OS, lines, follow, log.as_ref())?;
    let status = std::process::Command::new(plan.program)
        .args(&plan.args)
        .status()
        .with_context(|| format!("running {}", plan.program))?;
    if !status.success() {
        bail!("{} exited with {status}", plan.program);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_reads_the_user_journal_for_the_service_unit() {
        let plan = invocation("linux", 50, true, None).unwrap();
        assert_eq!(plan.program, "journalctl");
        assert_eq!(
            plan.args,
            [
                "--user",
                "-u",
                "forge-serve.service",
                "-n",
                "50",
                "--no-pager",
                "-f"
            ]
        );
        assert!(!invocation("linux", 5, false, None)
            .unwrap()
            .args
            .contains(&"-f".to_string()));
    }

    #[test]
    fn macos_tails_the_launchd_log_file() {
        let path = PathBuf::from("/data/serve.log");
        let plan = invocation("macos", 20, false, Some(&path)).unwrap();
        assert_eq!(plan.program, "tail");
        assert_eq!(plan.args, ["-n", "20", "/data/serve.log"]);
        assert!(invocation("macos", 20, false, None).is_err());
    }

    #[test]
    fn other_platforms_say_there_is_no_service_log() {
        let error = invocation("windows", 20, false, None)
            .unwrap_err()
            .to_string();
        assert!(error.contains("forge serve"), "{error}");
    }
}
