//! Command-line grammar for managed Forge Anywhere operations.

use clap::{Subcommand, ValueEnum};

/// Managed Forge Anywhere account and host operations.
#[derive(Subcommand)]
pub(crate) enum AnywhereCmd {
    /// Guided, resumable setup: sign in, recover or enroll, activate this host, and verify it.
    Setup {
        /// Stable name shown in the host fleet (defaults to the system hostname).
        #[arg(long, value_name = "NAME")]
        name: Option<String>,
        /// Recover with an offline Recovery Kit instead of approval from an enrolled device.
        #[arg(long)]
        recovery: bool,
    },
    /// Sign in with GitHub's device flow and enroll this controller device.
    Login {
        /// Recover with an offline Recovery Kit instead of approval from an enrolled device.
        #[arg(long)]
        recovery: bool,
    },
    /// List enrollment requests waiting for approval.
    Approvals,
    /// Approve a short-lived enrollment challenge from a new device.
    Approve {
        /// Challenge printed by `forge anywhere setup` on the new device.
        challenge: String,
        /// Approve without a prompt when the new device's signing key matches this one. Used by
        /// Forge Desktop on the same computer, which generated the key and so can vouch for it.
        #[arg(long, value_name = "KEY")]
        expect_signing_key: Option<String>,
    },
    /// Print the safety code this device computes for a waiting challenge, without approving.
    /// Used by `forge host add` so the code you compare is never taken from the new machine.
    #[command(hide = true)]
    SafetyCode {
        /// Challenge printed by `forge anywhere setup` on the new device.
        challenge: String,
    },
    /// Register this machine as a managed host and enable its connector.
    Enable {
        /// Stable name shown in the host fleet.
        #[arg(long, value_name = "NAME")]
        name: Option<String>,
    },
    /// Show local enrollment plus live entitlement, connection, and quota state.
    Status,
    /// Diagnose setup, enrollment, connector, and service readiness without printing secrets.
    Doctor,
    /// Move a paused session and its workspace capsule to another host.
    Handoff {
        /// Session id or unique prefix.
        session: String,
        /// Destination host id or unique name.
        #[arg(long, value_name = "HOST")]
        to: String,
    },
    /// Create an end-to-end encrypted replay link.
    Share {
        /// Session id or unique prefix.
        session: String,
        /// Link lifetime, capped at 30 days.
        #[arg(long, value_enum, default_value_t = ShareExpiry::Hours24)]
        expires: ShareExpiry,
    },
    /// Queue an encrypted create-session job for a host, even when its live relay is offline.
    Job {
        /// Destination host id, unique id prefix, or unique name.
        #[arg(long, value_name = "HOST")]
        to: String,
        /// Working directory on the destination host (encrypted end to end).
        #[arg(long, value_name = "PATH")]
        cwd: Option<String>,
        /// Optional session title (encrypted end to end).
        #[arg(long)]
        title: Option<String>,
        /// Optional model pin (encrypted end to end).
        #[arg(long)]
        model: Option<String>,
        /// Initial permission mode.
        #[arg(long, value_name = "MODE")]
        temper: Option<String>,
        /// Create the session in an isolated git worktree.
        #[arg(long)]
        worktree: bool,
    },
    /// Retry exact queued job ciphertext and poll categorical host acknowledgements.
    Jobs,
    /// List enrolled devices, or atomically revoke one and rotate the data-key epoch.
    Devices {
        /// Device id to revoke. Omit to list devices.
        #[arg(long, value_name = "DEVICE")]
        revoke: Option<String>,
    },
    /// Sync keys, skills, settings, MCP servers and instructions across your devices.
    Sync {
        #[command(subcommand)]
        cmd: Option<SyncCmd>,
    },
    /// Revoke this host and stop its managed connector. Local Forge is unchanged.
    Disable,
    /// Revoke local account tokens while preserving local Forge and encrypted history.
    Logout,
}

/// `forge anywhere sync` actions. Every category is per device and off until switched on here.
#[derive(Subcommand)]
pub(crate) enum SyncCmd {
    /// Show what syncs, what is pending, conflicts, skipped items and sign-ins to redo.
    Status,
    /// Start syncing a category on this device: keys, skills, settings, mcp, instructions.
    Enable {
        category: String,
        /// Skip the confirmation for keys (they upload credentials, end-to-end encrypted).
        #[arg(long)]
        yes: bool,
    },
    /// Stop syncing a category on this device. Already-synced local data is left in place.
    Disable { category: String },
    /// Ask the running daemon to run a sync cycle now.
    Now,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum ShareExpiry {
    #[value(name = "24h")]
    Hours24,
    #[value(name = "7d")]
    Days7,
    #[value(name = "30d")]
    Days30,
}
