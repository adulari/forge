//! `[terminal]`: which shell the app's terminal tabs start, and which shell the agent's own
//! `shell` tool runs commands through.

use serde::{Deserialize, Serialize};

/// The shell the agent's `shell` tool hands a command string to. `Auto` keeps the platform default
/// (`sh -c`, or `cmd /C` on Windows).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentShell {
    #[default]
    Auto,
    Sh,
    Bash,
    /// PowerShell 7 (`pwsh`).
    Pwsh,
    /// Windows PowerShell 5.1 (`powershell`).
    Powershell,
    Cmd,
}

impl AgentShell {
    /// The program and the arguments that precede the command string, or `None` for the platform
    /// default. PowerShell runs with `-NoProfile` so a user's profile cannot change what a command
    /// prints or prompt for input the agent cannot give.
    pub fn invocation(self) -> Option<(&'static str, &'static [&'static str])> {
        match self {
            Self::Auto => None,
            Self::Sh => Some(("sh", &["-c"])),
            Self::Bash => Some(("bash", &["-c"])),
            Self::Pwsh => Some(("pwsh", &["-NoProfile", "-NonInteractive", "-Command"])),
            Self::Powershell => {
                Some(("powershell", &["-NoProfile", "-NonInteractive", "-Command"]))
            }
            Self::Cmd => Some(("cmd", &["/C"])),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TerminalConfig {
    /// Program a new terminal tab runs. Unset uses `$SHELL` (`%COMSPEC%` on Windows).
    pub shell: Option<String>,
    /// Arguments passed to `shell`, for example `["-NoProfile"]`.
    pub shell_args: Vec<String>,
    /// Shell the agent's `shell` tool runs commands in: `auto`, `sh`, `bash`, `pwsh`,
    /// `powershell` or `cmd`.
    pub agent_shell: AgentShell,
    /// Start a session's first terminal when the session is created, so the dock opens with a
    /// shell already running. Skipped when `[host] max_terminals` is set.
    pub prewarm: bool,
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            shell: None,
            shell_args: Vec::new(),
            agent_shell: AgentShell::Auto,
            prewarm: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_block_keeps_the_platform_defaults() {
        let parsed: TerminalConfig = toml::from_str("").unwrap();
        assert_eq!(parsed, TerminalConfig::default());
        assert!(parsed.prewarm);
        assert_eq!(parsed.agent_shell.invocation(), None);
    }

    #[test]
    fn every_agent_shell_names_its_program_and_skips_profiles() {
        let parsed: TerminalConfig = toml::from_str(
            "shell = \"fish\"\nshell_args = [\"-l\"]\nagent_shell = \"pwsh\"\nprewarm = false",
        )
        .unwrap();
        assert_eq!(parsed.shell.as_deref(), Some("fish"));
        assert_eq!(parsed.shell_args, ["-l"]);
        assert!(!parsed.prewarm);
        let (program, args) = parsed.agent_shell.invocation().unwrap();
        assert_eq!(program, "pwsh");
        assert_eq!(args, ["-NoProfile", "-NonInteractive", "-Command"]);
        assert_eq!(AgentShell::Cmd.invocation(), Some(("cmd", &["/C"][..])));
        assert!(toml::from_str::<TerminalConfig>("agent_shell = \"zsh\"").is_err());
    }
}
