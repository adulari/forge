//! Replacing this process with the tool the conversation moves to.

use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result};

use super::cc_fs;

/// What to run, where.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Launch {
    pub(crate) program: PathBuf,
    pub(crate) args: Vec<String>,
    pub(crate) dir: Option<PathBuf>,
}

impl Launch {
    /// `forge chat --resume <id>` from this very binary, with the permission mode Claude Code
    /// was in.
    pub(crate) fn forge(
        forge_id: &str,
        cc_permission_mode: Option<&str>,
        dir: &str,
    ) -> Result<Self> {
        let mut args = vec!["chat".to_string(), "--resume".into(), forge_id.into()];
        if let Some(mode) = cc_permission_mode.and_then(cc_fs::forge_mode_for) {
            args.extend(["--mode".to_string(), mode.into()]);
        }
        Ok(Self {
            program: std::env::current_exe().context("locating the forge executable")?,
            args,
            dir: Some(PathBuf::from(dir)),
        })
    }

    /// `claude --resume <id>` in the session's directory (where Claude Code looks for it).
    pub(crate) fn claude(
        cc_session_id: &str,
        model: Option<&str>,
        forge_permission_mode: &str,
        dir: &str,
    ) -> Self {
        let mut args = vec!["--resume".to_string(), cc_session_id.into()];
        if let Some(m) = model {
            args.extend(["--model".to_string(), m.into()]);
        }
        args.extend(
            cc_fs::claude_mode_flags(forge_permission_mode)
                .into_iter()
                .map(String::from),
        );
        #[cfg(windows)]
        {
            args.splice(0..0, ["/C".to_string(), "claude".into()]);
            return Self {
                program: PathBuf::from("cmd"),
                args,
                dir: Some(PathBuf::from(dir)),
            };
        }
        #[cfg(not(windows))]
        Self {
            program: PathBuf::from("claude"),
            args,
            dir: Some(PathBuf::from(dir)),
        }
    }

    /// The command as a person would type it, for dry runs.
    pub(crate) fn display(&self) -> String {
        let program = self.program.file_name().map_or_else(
            || self.program.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        let shown = if program == "forge" || program.starts_with("forge") {
            "forge".to_string()
        } else {
            program
        };
        format!("{shown} {}", self.args.join(" "))
    }

    fn command(&self) -> Command {
        let mut cmd = Command::new(&self.program);
        cmd.args(&self.args);
        if let Some(dir) = self.dir.as_ref().filter(|d| d.is_dir()) {
            cmd.current_dir(dir);
        }
        cmd
    }

    /// Become the target. On Unix this is `exec` and only returns on failure; elsewhere the child
    /// runs to completion and this process exits with its status.
    pub(crate) fn run(self) -> Result<()> {
        let mut cmd = self.command();
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            let err = cmd.exec();
            Err(err).with_context(|| format!("starting {}", self.display()))
        }
        #[cfg(not(unix))]
        {
            let status = cmd
                .status()
                .with_context(|| format!("starting {}", self.display()))?;
            std::process::exit(status.code().unwrap_or(1));
        }
    }
}
