//! Arguments of `forge report` (shared with the completion/man-page facade in `lib.rs`).

#[derive(Debug, clap::Args)]
pub struct ReportArgs {
    /// Print the most recent report exactly as it would be sent.
    #[arg(long)]
    pub(crate) show_last: bool,
    /// List what has been sent from this machine.
    #[arg(long)]
    pub(crate) sent: bool,
    /// List reports waiting for a decision (ask mode).
    #[arg(long)]
    pub(crate) pending: bool,
    /// Send every report waiting for a decision.
    #[arg(long)]
    pub(crate) send_pending: bool,
    /// Set the mode: auto, ask or off.
    #[arg(long)]
    pub(crate) mode: Option<String>,
    /// Report a problem yourself; a preview is shown before anything is sent.
    #[arg(long, short = 'm')]
    pub(crate) message: Option<String>,
    /// Send a --message report without asking for confirmation.
    #[arg(long, short = 'y')]
    pub(crate) yes: bool,
    /// Raise a deliberate `panic` or `internal` error to verify the reporting pipeline
    /// end to end (the text comes from --message).
    #[arg(long, hide = true)]
    pub(crate) selftest: Option<String>,
}
