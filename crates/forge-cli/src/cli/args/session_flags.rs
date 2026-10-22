//! Claude Code flags that shape a session, whatever surface runs it. Flattened into `forge run`
//! today; built so `forge chat` can flatten the same struct (docs/features/claude-code-parity.md).

use std::path::PathBuf;

use clap::{Args, ValueEnum};

use super::Mode;

/// Claude Code's `--permission-mode` spellings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum ClaudePermissionMode {
    #[value(name = "default")]
    Default,
    #[value(name = "acceptEdits")]
    AcceptEdits,
    #[value(name = "bypassPermissions")]
    BypassPermissions,
    #[value(name = "plan")]
    Plan,
    #[value(name = "auto")]
    Auto,
    /// Never prompt, even at a terminal; anything not pre-approved is denied. The rules are
    /// `default`'s, so a run without a terminal behaves the same either way.
    #[value(name = "dontAsk")]
    DontAsk,
}

impl From<ClaudePermissionMode> for Mode {
    fn from(mode: ClaudePermissionMode) -> Self {
        match mode {
            ClaudePermissionMode::Default | ClaudePermissionMode::DontAsk => Mode::Default,
            ClaudePermissionMode::AcceptEdits => Mode::AcceptEdits,
            ClaudePermissionMode::BypassPermissions => Mode::Bypass,
            ClaudePermissionMode::Plan => Mode::Plan,
            ClaudePermissionMode::Auto => Mode::Auto,
        }
    }
}

/// Claude Code's `--effort` levels. `max` is the highest reasoning rung (Forge's `xhigh`; no
/// provider has a knob above it); `ultracode` also lets the agent orchestrate through workflows,
/// which is Forge's `whitehot`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum EffortArg {
    Low,
    Medium,
    High,
    Xhigh,
    Max,
    #[value(alias = "whitehot")]
    Ultracode,
}

impl From<EffortArg> for forge_types::EffortLevel {
    fn from(effort: EffortArg) -> Self {
        use forge_types::EffortLevel;
        match effort {
            EffortArg::Low => EffortLevel::Low,
            EffortArg::Medium => EffortLevel::Medium,
            EffortArg::High => EffortLevel::High,
            EffortArg::Xhigh | EffortArg::Max => EffortLevel::XHigh,
            EffortArg::Ultracode => EffortLevel::WhiteHot,
        }
    }
}

#[derive(Args, Clone, Debug, Default)]
pub(crate) struct SessionFlags {
    /// Claude Code's spelling of `--mode`: default, acceptEdits, bypassPermissions, plan, auto or
    /// dontAsk.
    #[arg(long, value_enum, value_name = "MODE", conflicts_with = "mode")]
    pub(crate) permission_mode: Option<ClaudePermissionMode>,
    /// Same as `--mode bypass`.
    #[arg(
        long = "dangerously-skip-permissions",
        conflicts_with_all = ["mode", "permission_mode"]
    )]
    pub(crate) dangerously_skip_permissions: bool,
    /// Tools to allow without asking, Claude Code syntax: `"Bash(git log:*) Read"`. Space or comma
    /// separated; repeat the flag for more.
    #[arg(
        long = "allowedTools",
        visible_alias = "allowed-tools",
        value_name = "TOOLS"
    )]
    pub(crate) allowed_tools: Vec<String>,
    /// Tools to deny outright, same syntax as `--allowedTools`.
    #[arg(
        long = "disallowedTools",
        visible_alias = "disallowed-tools",
        value_name = "TOOLS"
    )]
    pub(crate) disallowed_tools: Vec<String>,
    /// Append standing system instructions for the provider without including them in mesh
    /// task classification. Repeat the flag to add multiple system messages.
    #[arg(
        long = "system",
        visible_alias = "append-system-prompt",
        value_name = "TEXT"
    )]
    pub(crate) system: Vec<String>,
    /// Append the contents of a file as a system message (like `--append-system-prompt`).
    #[arg(long, value_name = "FILE")]
    pub(crate) append_system_prompt_file: Vec<PathBuf>,
    /// Replace Forge's base coding-agent prompt with this text for every model (the environment
    /// block stays; bridged CLIs such as `claude-cli::` still run on their own built-in prompt).
    #[arg(long, value_name = "TEXT", conflicts_with = "system_prompt_file")]
    pub(crate) system_prompt: Option<String>,
    /// Like `--system-prompt`, read from a file.
    #[arg(long, value_name = "FILE")]
    pub(crate) system_prompt_file: Option<PathBuf>,
    /// Extra directories the file tools may read and edit (adds to `[tools] extra_roots` for this
    /// run). Each must exist.
    #[arg(long, value_name = "DIR", num_args = 1..)]
    pub(crate) add_dir: Vec<PathBuf>,
    /// Reasoning effort for this session: low, medium, high, xhigh, max or ultracode. Sets the same
    /// ceiling as `/effort` (`max` = xhigh, `ultracode` = whitehot).
    #[arg(long, value_enum, value_name = "LEVEL")]
    pub(crate) effort: Option<EffortArg>,
    /// Start a new session with this id (a UUID that is not already in use).
    #[arg(long, value_name = "UUID", conflicts_with_all = ["continue", "resume"])]
    pub(crate) session_id: Option<String>,
    /// A display name for the session, shown in `forge sessions` and the resume picker.
    #[arg(short = 'n', long, value_name = "NAME")]
    pub(crate) name: Option<String>,
    /// MCP servers for this run from JSON files or inline JSON strings (Claude Code's `.mcp.json`
    /// shape: `{"mcpServers": {...}}`), added to the configured servers.
    #[arg(long, value_name = "CONFIG", num_args = 1..)]
    pub(crate) mcp_config: Vec<String>,
    /// Use only the servers from `--mcp-config`, ignoring every configured MCP server.
    #[arg(long, requires = "mcp_config")]
    pub(crate) strict_mcp_config: bool,
    /// Which config layers to load, comma separated: user, project, local (`local` and `project`
    /// both mean the project's `.forge/`). Omit for all of them.
    #[arg(long, value_name = "SOURCES")]
    pub(crate) setting_sources: Option<String>,
    /// Models to fail over to, in order, when the routed model is unavailable (comma separated).
    /// With `--model` they are the only models the run may fall to.
    #[arg(long, value_name = "MODELS")]
    pub(crate) fallback_model: Option<String>,
}

impl SessionFlags {
    /// The permission mode these flags and `--mode` ask for together.
    pub(crate) fn effective_mode(&self, mode: Option<Mode>) -> Option<Mode> {
        mode.or(self.permission_mode.map(Mode::from))
            .or(self.dangerously_skip_permissions.then_some(Mode::Bypass))
    }

    /// Whether any flag changes the session in a way a daemon-hosted run could not honour, so the
    /// run must stay in this process.
    pub(crate) fn needs_local_session(&self) -> bool {
        self.system_prompt.is_some()
            || self.system_prompt_file.is_some()
            || !self.append_system_prompt_file.is_empty()
            || !self.system.is_empty()
            || !self.add_dir.is_empty()
            || self.effort.is_some()
            || self.session_id.is_some()
            || self.name.is_some()
            || !self.mcp_config.is_empty()
            || self.setting_sources.is_some()
            || self.fallback_model.is_some()
            || !self.allowed_tools.is_empty()
            || !self.disallowed_tools.is_empty()
    }
}
