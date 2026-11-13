use super::Scope;
use clap::{Subcommand, ValueEnum};

#[derive(Subcommand)]
pub(crate) enum SkillCmd {
    /// Install a skill pack from a GitHub repository (owner/repo[@ref]) or HTTPS URL.
    ///
    /// Fetches every `.md` file from the repo root (or a `skills/` subdir if one exists) and
    /// writes them into the user skills directory (`~/.config/forge/skills/`).
    ///
    /// See also: `forge plugin install` is the canonical, marketplace-aware pack installer
    /// (`forge plugin install owner/repo [--marketplace <name>]`); this `skill install` is the
    /// simpler GitHub/URL fetcher. Both land packs in the same skills directory.
    ///
    /// Examples:
    ///   forge skill install anthropics/forge-skills
    ///   forge skill install anthropics/forge-skills@main
    Install {
        /// GitHub `owner/repo[@ref]` shorthand or a full HTTPS URL.
        url: String,
    },
    /// Distil a past session's transcript into a reusable Forge skill.
    ///
    /// Reads the persisted transcript of SESSION_ID, calls a cheap model to synthesise a
    /// SKILL.md (a generalised, reusable methodology), and writes it where Forge discovers skills.
    /// The new skill is immediately invokable in future sessions.
    ///
    /// Examples:
    ///   forge skill from-session abc123
    ///   forge skill from-session abc123 --name refactor-workflow --scope project
    FromSession {
        /// Session id (or unambiguous prefix) — see `forge sessions`.
        session_id: String,
        /// Override the skill slug (kebab-case, e.g. `refactor-workflow`).
        /// Derived from the session's first user prompt when absent.
        #[arg(long)]
        name: Option<String>,
        /// Where to write the skill: `user` (default), `project`, or `local` (`.forge/skills/`).
        #[arg(long, default_value = "user")]
        scope: Scope,
    },
    /// Export this machine's Forge skills, commands, and agents to a portable directory — the
    /// inverse of `forge import`. Writes `<dest>/commands`, `<dest>/skills`, `<dest>/agents` in the
    /// same layout import reads, so the bundle can be copied to another machine or shared. Existing
    /// files in the destination are kept (never overwritten).
    ///
    /// Examples:
    ///   forge skill export ./my-forge-skills
    ///   forge skill export /tmp/bundle --scope project
    Export {
        /// Destination directory (created if absent).
        dest: std::path::PathBuf,
        /// Which scope to export: `user`, `project`, or `all` (default — the effective merged set).
        #[arg(long, default_value = "all")]
        scope: ExportScope,
    },
    /// Load a Forge bundle (a directory produced by `forge skill export`) into your skills —
    /// the inverse of export, closing the round-trip. Alias: `load`.
    ///
    /// NOT to be confused with `forge import` (which pulls commands/skills from ANOTHER AI CLI —
    /// Claude Code, Codex, Cursor, Aider); `forge skill import` loads a Forge-native bundle.
    /// Reads `<src>/{commands,skills,agents}` and merges them via the real catalog readers
    /// (malformed files skipped + warned); anything already present is kept, never overwritten.
    ///
    /// Examples:
    ///   forge skill import ./my-forge-skills
    ///   forge skill load /tmp/bundle --scope project
    #[command(alias = "load")]
    Import {
        /// Source bundle directory (as written by `forge skill export`).
        src: std::path::PathBuf,
        /// Where to import: `user` (default), `project`, or `local` (`.forge/`).
        #[arg(long, default_value = "user")]
        scope: Scope,
    },
    /// Re-run path and binary normalization on all installed skills and commands in-place:
    /// replaces legacy `~/.claude/` paths and `claude`/`codex` command references with their
    /// Forge equivalents. Safe to run multiple times — only writes files that actually changed.
    ///
    /// Examples:
    ///   forge skill normalize
    ///   forge skill normalize --scope project
    Normalize {
        /// Which skills to normalize: `user` (default) or `project`/`local` (`.forge/`).
        #[arg(long, value_enum)]
        scope: Option<Scope>,
        /// Legacy alias for `--scope project` (hidden; kept for back-compat).
        #[arg(long, hide = true, conflicts_with = "scope")]
        project: bool,
    },
    /// Re-fetch installed skill packs and update them in place, using the source recorded in the
    /// lockfile (`~/.config/forge/installed-skills.toml`). With NAME, update only that pack;
    /// without, update every recorded pack. Honors the pinned `@ref`/tag from install time.
    ///
    /// Examples:
    ///   forge skill update
    ///   forge skill update my-pack
    Update {
        /// Installed pack name to update (omit to update all).
        name: Option<String>,
    },
}

#[derive(Clone, Copy, ValueEnum, PartialEq, Eq)]
pub(crate) enum ExportScope {
    User,
    Project,
    All,
}
