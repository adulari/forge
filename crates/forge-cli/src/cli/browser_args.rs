//! Clap definition for `forge browser`. Its own file because `cli/args.rs` is also compiled into
//! the library facade, which has no `cli::commands`.

use clap::Subcommand;

#[derive(Subcommand)]
pub(crate) enum BrowserOp {
    /// Launch a Chromium-family browser with remote debugging on a dedicated, persistent Forge
    /// profile (log in once), then print the URL to put in `[browser] attach`.
    Attach {
        /// Remote-debugging port.
        #[arg(long, default_value_t = forge_browser::attach::DEFAULT_ATTACH_PORT)]
        port: u16,
        /// Profile directory (default: `<data dir>/forge/browser-attach-profile`). Never the
        /// browser's default profile — Chrome 136+ refuses debugging there.
        #[arg(long)]
        profile: Option<std::path::PathBuf>,
        /// Browser executable (default: first of Chrome, Chromium, Brave, Edge found).
        #[arg(long)]
        browser: Option<std::path::PathBuf>,
        /// Print the command instead of launching it.
        #[arg(long)]
        print: bool,
    },
}
