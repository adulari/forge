//! Wiring for automatic issue reports (`forge-reporting`) and the `forge report` command.
//! See `docs/features/issue-reporting.md`.

use std::io::{BufRead, IsTerminal, Write};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{bail, Context, Result};
use forge_reporting::{ErrorKind, HostInfo, Incident, ReportingMode, Surface};

use crate::report_args::ReportArgs;
use crate::update::{detect_install_method, InstallMethod};
use crate::Command;

fn surface_for(command: &Command) -> Surface {
    match command {
        Command::Serve { .. } => Surface::Daemon,
        Command::Chat { plain: false, .. } => Surface::Tui,
        Command::Run { tui: true, .. } => Surface::Tui,
        Command::Run { .. } => Surface::Headless,
        _ => Surface::Cli,
    }
}

/// Install the panic hook and tell the reporting crate who is running. Cheap: no network, no
/// config read beyond the mode (which `init` caches).
pub(crate) fn start(command: &Command) {
    forge_reporting::install_panic_hook();
    forge_reporting::init(HostInfo {
        surface: surface_for(command),
        version: env!("CARGO_PKG_VERSION").to_string(),
        git_sha: option_env!("FORGE_GIT_SHA").map(str::to_string),
        install_method: Some(
            match detect_install_method() {
                InstallMethod::Binary => "binary",
                InstallMethod::Homebrew => "homebrew",
                InstallMethod::Cargo => "cargo",
            }
            .to_string(),
        ),
    });
    if forge_reporting::mode() == ReportingMode::Off {
        return;
    }
    // Known provider keys are only worth redacting once a report exists; build the redactor off
    // the startup path (keyring lookups are D-Bus round trips). The built-in secret shapes cover
    // the gap.
    let _ = std::thread::Builder::new()
        .name("forge-report-redactor".into())
        .spawn(|| {
            let redactor = forge_core::redact::Redactor::from_environment();
            forge_reporting::set_redactor(move |text| redactor.redact(text).into_owned());
        });
}

/// Report an error that ended a command if (and only if) it is one of Forge's own bugs.
pub(crate) fn report_top_level(error: &anyhow::Error) {
    for cause in error.chain() {
        if let Some(store) = cause.downcast_ref::<forge_store::StoreError>() {
            if !matches!(store, forge_store::StoreError::SchemaTooNew { .. }) {
                forge_reporting::report(Incident::gray(ErrorKind::Store, "forge_cli::main", store));
            }
            return;
        }
        if let Some(forge_core::CoreError::Internal(message)) =
            cause.downcast_ref::<forge_core::CoreError>()
        {
            forge_reporting::report(Incident::bug(
                ErrorKind::Internal,
                "forge_cli::main",
                message,
            ));
            return;
        }
    }
}

/// Deliver anything queued before the process exits, within a short budget.
pub(crate) fn finish() {
    let _ = forge_reporting::flush_blocking(std::time::Duration::from_secs(2));
}

static SEEN_PENDING: AtomicU64 = AtomicU64::new(u64::MAX);
static SEEN_REPORTED: AtomicU64 = AtomicU64::new(0);

/// Per-frame TUI poll: open the ask-mode card for a held report, and leave a quiet note when an
/// automatic report went out. Cheap (two atomics) unless something changed.
pub(crate) fn poll(app: &mut forge_tui::App) -> bool {
    let mut changed = false;
    let reported = forge_reporting::reported_seq();
    if SEEN_REPORTED.swap(reported, Ordering::Relaxed) != reported
        && forge_reporting::mode() == ReportingMode::Auto
    {
        app.note("◇ reported an internal error (details: /bug, `forge report --show-last`)");
        changed = true;
    }
    let pending = forge_reporting::pending_seq();
    // First poll (u64::MAX) also looks, to catch a report held by an earlier crashed run.
    if SEEN_PENDING.swap(pending, Ordering::Relaxed) != pending
        && !app.picker.open
        && forge_reporting::mode() == ReportingMode::Ask
    {
        changed |= open_card(app);
    }
    changed
}

fn open_card(app: &mut forge_tui::App) -> bool {
    let Some(item) = forge_reporting::pending().into_iter().next() else {
        return false;
    };
    show_card(app, &item.id, &item.report);
    true
}

fn show_card(app: &mut forge_tui::App, id: &str, report: &forge_reporting::Report) {
    let preview = report.preview();
    let lines: Vec<&str> = preview.lines().collect();
    let shown = lines
        .iter()
        .take(60)
        .copied()
        .collect::<Vec<_>>()
        .join("\n");
    let more = if lines.len() > 60 { "\n..." } else { "" };
    app.push_scrollback_text(&format!(
        "This is exactly what would be sent. It never includes prompts, replies, code, paths or keys.\n{shown}{more}"
    ));
    let mut rows = vec![
        forge_tui::PickerRow {
            id: format!("send@{id}"),
            title: "Send".to_string(),
            subtitle: "report it to the maintainers".to_string(),
        },
        forge_tui::PickerRow {
            id: format!("skip@{id}"),
            title: "Don't send".to_string(),
            subtitle: "discard this report".to_string(),
        },
    ];
    if !report.is_manual() {
        rows.push(forge_tui::PickerRow {
            id: format!("always@{id}"),
            title: "Always send".to_string(),
            subtitle: "send this and report automatically from now on".to_string(),
        });
    }
    app.picker.open_with(
        forge_tui::PickerKind::ErrorReport,
        &format!("report: {}", report.headline()),
        rows,
    );
}

/// `/bug <description>`: build a manual report and show it on the card.
pub(crate) fn bug_card(app: &mut forge_tui::App, text: &str) {
    if forge_reporting::forced_off() {
        app.note("reporting is disabled by FORGE_NO_REPORTS");
        return;
    }
    let text = text.trim();
    if text.is_empty() {
        app.note("usage: /bug <what went wrong>  (a preview is shown before anything is sent)");
        return;
    }
    let report = forge_reporting::build_manual(text, None);
    match forge_reporting::hold_manual(&report) {
        Some(id) => show_card(app, &id, &report),
        None => app.note("could not prepare the report (no data directory)"),
    }
}

/// Enter on a card row.
pub(crate) fn resolve_card(app: &mut forge_tui::App, row_id: &str) {
    let Some((action, id)) = row_id.split_once('@') else {
        return;
    };
    let decision = match action {
        "send" => forge_reporting::Decision::Send,
        "always" => forge_reporting::Decision::AlwaysSend,
        _ => forge_reporting::Decision::Skip,
    };
    forge_reporting::resolve(id, decision);
    app.note(match decision {
        forge_reporting::Decision::Skip => "report discarded",
        forge_reporting::Decision::Send => "report sent (queued; delivered in the background)",
        forge_reporting::Decision::AlwaysSend => {
            "report sent; error reports are now automatic (change with /config)"
        }
    });
    if !app.picker.open {
        open_card(app);
    }
}

/// `forge report --selftest panic|internal`: raise a real panic or a classified internal error so
/// the whole pipeline (capture, scrub, queue, relay, issue) can be checked on a deployment.
pub(crate) fn selftest(kind: &str, text: &str) -> Result<()> {
    match kind {
        "panic" => panic!("{text}"),
        "internal" => {
            let disposition = forge_reporting::report(Incident::bug(
                ErrorKind::Internal,
                "forge_cli::selftest",
                text,
            ));
            let stats = forge_reporting::flush_blocking(std::time::Duration::from_secs(5));
            println!("selftest: {disposition:?}, sent {}", stats.sent);
            Ok(())
        }
        other => bail!("unknown selftest `{other}` (use panic or internal)"),
    }
}

/// `forge report`.
pub(crate) fn run(args: ReportArgs) -> Result<()> {
    let ReportArgs {
        show_last,
        sent,
        pending,
        send_pending,
        mode,
        message,
        yes,
        selftest: _,
    } = args;
    if let Some(raw) = mode {
        let mode = ReportingMode::parse(&raw)
            .with_context(|| format!("unknown mode `{raw}` (use auto, ask or off)"))?;
        forge_reporting::set_mode(mode)?;
        println!("error reports: {}", mode.as_str());
        return Ok(());
    }
    if show_last {
        match forge_reporting::last_report() {
            Some(report) => println!("{}", report.preview()),
            None => println!("No report has been built on this machine yet."),
        }
        return Ok(());
    }
    if sent {
        let log = forge_reporting::sent_log();
        if log.is_empty() {
            println!("Nothing has been sent.");
        }
        for entry in log {
            let when =
                chrono::DateTime::from_timestamp(i64::try_from(entry.sent_at).unwrap_or(0), 0)
                    .map(|t| t.format("%Y-%m-%d %H:%M UTC").to_string())
                    .unwrap_or_default();
            println!("{when}  {}", entry.report.headline());
        }
        return Ok(());
    }
    if pending || send_pending {
        let held = forge_reporting::pending();
        if held.is_empty() {
            println!("No reports are waiting for a decision.");
        }
        for item in held {
            println!("{}\n{}\n", item.report.headline(), item.report.preview());
            if send_pending {
                forge_reporting::resolve(&item.id, forge_reporting::Decision::Send);
                println!("-> queued for sending");
            }
        }
        if send_pending {
            let _ = forge_reporting::flush_blocking(std::time::Duration::from_secs(5));
        }
        return Ok(());
    }
    if let Some(note) = message {
        return manual(&note, yes);
    }
    println!(
        "Error reports: {}\n\n  forge report --show-last       the last report, exactly as it would be sent\n  forge report --sent            what has been sent from this machine\n  forge report --pending         reports waiting for your decision (ask mode)\n  forge report --message \"...\"   report a problem yourself (shows a preview first)\n  forge report --mode auto|ask|off",
        forge_reporting::mode().as_str()
    );
    Ok(())
}

fn manual(note: &str, yes: bool) -> Result<()> {
    if forge_reporting::forced_off() {
        bail!("reporting is disabled by FORGE_NO_REPORTS");
    }
    let report = forge_reporting::build_manual(note, None);
    println!(
        "This is exactly what will be sent:\n\n{}\n",
        report.preview()
    );
    if !yes {
        if !std::io::stdin().is_terminal() {
            bail!("not a terminal: re-run with --yes to send");
        }
        print!("Send this report? [y/N] ");
        std::io::stdout().flush().ok();
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line)?;
        if !matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
            println!("Not sent.");
            return Ok(());
        }
    }
    forge_reporting::submit_manual(report).map_err(anyhow::Error::msg)?;
    let stats = forge_reporting::flush_blocking(std::time::Duration::from_secs(5));
    if stats.sent > 0 {
        println!("Sent. Thank you.");
    } else {
        println!("Queued; it will be sent when the relay is reachable.");
    }
    Ok(())
}
