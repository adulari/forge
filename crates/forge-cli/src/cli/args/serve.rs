//! Command-line grammar for `forge serve pair` and `forge serve clients`.

use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub(crate) enum ServeCmd {
    /// Mint a single-use link that pairs one new device. The link stops working if it is not
    /// opened within `--ttl`; after that the device keeps its own token until it is revoked.
    Pair {
        /// How long the link stays valid before its first use (`90s`, `10m`, `2h`).
        #[arg(long, default_value = "10m")]
        ttl: String,
        /// `full` (default: everything the owner can do) or `viewer` (read-only).
        #[arg(long)]
        preset: Option<String>,
        /// Name shown in the paired-devices list.
        #[arg(long)]
        label: Option<String>,
    },
    /// List or revoke paired devices.
    Clients {
        #[command(subcommand)]
        cmd: ClientsCmd,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum ClientsCmd {
    /// Show every paired device with its scopes and when it last connected.
    List,
    /// Cut one device off (id or unambiguous prefix); every other device keeps working.
    Revoke {
        /// Device id from `forge serve clients list`.
        id: String,
    },
}
