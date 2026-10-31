//! Command-line grammar for `forge host`: adding and listing other machines.

use clap::Subcommand;

/// Add another machine to your Forge fleet over SSH, or list the ones added.
#[derive(Subcommand)]
pub(crate) enum HostCmd {
    /// Install Forge on a machine over SSH, start it as a service and pair it to your Forge
    /// Anywhere fleet. Uses your own ssh configuration and keys; nothing is stored on the
    /// remote besides Forge itself.
    ///
    /// Example:
    ///   forge host add dev@build-box.example.com --name build-box
    Add {
        /// `[user@]host` exactly as you would pass it to ssh (an ssh config alias works).
        target: String,
        /// Name the machine shows under in the fleet (defaults to its own hostname).
        #[arg(long)]
        name: Option<String>,
        /// Install the service on the LAN (`forge service install --lan`) instead of loopback
        /// only. The managed Anywhere connector does not need it.
        #[arg(long)]
        lan: bool,
        /// Port for the remote daemon (defaults to the remote's configured port).
        #[arg(long)]
        port: Option<u16>,
        /// Stop after the service is running; do not pair it to Forge Anywhere.
        #[arg(long)]
        no_pair: bool,
        /// Approve the new device without typing APPROVE. The request still has to be created
        /// by the machine you just named, but you give up the safety-code comparison.
        #[arg(long)]
        yes: bool,
        /// Extra argument for ssh, repeatable (for example `--ssh-arg=-p --ssh-arg=2222`).
        #[arg(long = "ssh-arg", value_name = "ARG", allow_hyphen_values = true)]
        ssh_args: Vec<String>,
        /// Run this local installer script on the remote instead of downloading the published
        /// one there (for machines that cannot reach GitHub).
        #[arg(long, value_name = "FILE")]
        installer: Option<String>,
    },
    /// List the machines added with `forge host add`.
    List,
    /// Forget a machine added with `forge host add`. The remote keeps running; revoke it with
    /// `forge anywhere devices --revoke` if it should lose access.
    Remove {
        /// The name shown by `forge host list`.
        name: String,
    },
}
