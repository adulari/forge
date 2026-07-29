//! Command-line grammar for marketplace-aware plugin lifecycle operations.

use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub(crate) enum PluginMarketplaceCmd {
    /// Register a marketplace: a name → source mapping. SOURCE is a GitHub `owner/repo` (whose
    /// top-level directories are packages), a full git URL, or an `owner/repo` index repo.
    ///
    /// Examples:
    ///   forge plugin marketplace add community anthropics/forge-marketplace
    ///   forge plugin marketplace add internal https://git.corp/ai/skills.git --ref main
    Add {
        /// Marketplace name used in `forge plugin install <pkg>@<name>`.
        name: String,
        /// Source: `owner/repo`, a full git URL, or an index repo.
        source: String,
        /// Pin the marketplace to a branch/tag.
        #[arg(long, name = "ref")]
        ref_: Option<String>,
    },
    /// List configured marketplace sources.
    List,
    /// Remove a marketplace source.
    Remove { name: String },
}

#[derive(Subcommand, Debug)]
pub(crate) enum PluginCmd {
    /// Install a skill pack. PLUGIN is `owner/repo[@ref]`, a full git URL, `pkg@marketplace`, or a
    /// bare `pkg` resolved against `--marketplace`. Records a lockfile entry for `forge plugin
    /// update`. Honors `GITHUB_TOKEN` for private repos. Alias: `add`.
    ///
    /// This is the canonical, marketplace-aware pack installer. The simpler `forge skill install`
    /// is a plain GitHub/URL fetcher that lands packs in the same skills directory.
    #[command(alias = "add")]
    Install {
        plugin: String,
        /// Resolve PLUGIN as a package within this registered marketplace.
        #[arg(long)]
        marketplace: Option<String>,
    },
    List {
        /// Include remotely available packages from configured marketplaces.
        #[arg(long)]
        available: bool,
    },
    /// Remove an installed skill pack.
    Remove { plugin: String },
    /// Re-fetch installed packs and update them. With PLUGIN, update only that pack.
    Update { plugin: Option<String> },
    /// Manage plugin marketplaces.
    Marketplace {
        #[command(subcommand)]
        cmd: PluginMarketplaceCmd,
    },
}
