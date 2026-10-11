//! Command-line grammar for `forge import <source>`.

use super::Scope;
use clap::Subcommand;

/// `forge import <source>` pulls commands/skills FROM ANOTHER AI CLI (Claude Code, Codex, Cursor,
/// Aider) into Forge. For loading a Forge-native bundle made by `forge skill export`, use
/// `forge skill import` / `forge skill load` instead.
#[derive(Subcommand)]
pub(crate) enum ImportSource {
    /// Copy `~/.claude/commands/*.md` and `~/.claude/skills/*/` into Forge (user scope by
    /// default). Existing definitions are kept; malformed files are skipped.
    Claude {
        /// Where to import: `user` (default), `project`, or `local` (`./.forge`).
        #[arg(long, value_enum)]
        scope: Option<Scope>,
        /// Legacy alias for `--scope project` (hidden; kept for back-compat).
        #[arg(long, hide = true, conflicts_with = "scope")]
        project: bool,
        /// Also import `settings.json` hooks (written to Forge's CC-compatible `settings.json`,
        /// replacing any `hooks` already there). Off by default: hook scripts are executable code
        /// written against Claude Code's behaviour, so they are opt-in.
        #[arg(long)]
        hooks: bool,
    },
    /// Move a Claude Code session into Forge with its full history, to keep going on the
    /// claude-cli bridge. Prints the new Forge session id on stdout (the report goes to stderr),
    /// so `forge chat --resume "$(forge import claude-session <id> --quiet)"` opens it. The
    /// model context is what Claude Code would send next: its latest compact summary plus
    /// everything after it; earlier turns stay in history and replay but are not sent.
    ClaudeSession {
        /// A Claude Code session id (looked up under `~/.claude/projects`) or the path to its
        /// `.jsonl` transcript.
        session: String,
        /// Model to pin the session to (`claude-cli::<model>`, or a bare model id). Defaults to
        /// the model of the transcript's last assistant message.
        #[arg(long)]
        model: Option<String>,
        /// Session title. Defaults to Claude Code's own name for the session, else the first prompt.
        #[arg(long)]
        title: Option<String>,
        /// Workspace for the new session. Defaults to the transcript's working directory.
        #[arg(long)]
        cwd: Option<std::path::PathBuf>,
        /// Print only the session id (no report on stderr).
        #[arg(long)]
        quiet: bool,
    },
    /// Copy Codex CLI custom prompts (`~/.codex/prompts/*.md`) into Forge as commands (user
    /// scope by default). Existing definitions are kept; malformed files are skipped.
    Codex {
        /// Where to import: `user` (default), `project`, or `local` (`./.forge`).
        #[arg(long, value_enum)]
        scope: Option<Scope>,
        /// Legacy alias for `--scope project` (hidden; kept for back-compat).
        #[arg(long, hide = true, conflicts_with = "scope")]
        project: bool,
    },
    /// Import Cursor AI rules (`~/.cursor/rules/*.mdc`) into Forge as commands. Each `.mdc`
    /// rule becomes one command; `globs`/`alwaysApply` fields are dropped — only `description`
    /// and the rule body are kept. Existing commands are not overwritten.
    Cursor {
        /// Where to import: `user` (default), `project`, or `local` (`./.forge`).
        #[arg(long, value_enum)]
        scope: Option<Scope>,
        /// Legacy alias for `--scope project` (hidden; kept for back-compat).
        #[arg(long, hide = true, conflicts_with = "scope")]
        project: bool,
    },
    /// Import Aider AI convention files into Forge as commands. Looks for
    /// `CONVENTIONS.md` / `.aider.md` / `.aider.conventions.md` in `~` and then `$PWD`.
    /// Each file becomes one command named after the file. Existing commands are not overwritten.
    Aider {
        /// Where to import: `user` (default), `project`, or `local` (`./.forge`).
        #[arg(long, value_enum)]
        scope: Option<Scope>,
        /// Legacy alias for `--scope project` (hidden; kept for back-compat).
        #[arg(long, hide = true, conflicts_with = "scope")]
        project: bool,
    },
}
