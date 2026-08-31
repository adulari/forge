//! Environment diagnostics for `forge doctor`.

use std::io::IsTerminal;

use crate::doctor::{check, Check, Status};

pub(crate) fn environment_checks() -> Vec<Check> {
    let mut out = Vec::new();

    // git
    let git = binary_on_path("git");
    out.push(check(
        if git { Status::Ok } else { Status::Warn },
        "git",
        if git { "on PATH" } else { "not found" },
        (!git).then_some("install git — some features (provenance, /init) use it"),
    ));
    if git {
        let in_repo = std::process::Command::new("git")
            .args(["rev-parse", "--is-inside-work-tree"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        out.push(check(
            Status::Info,
            "git repo",
            if in_repo {
                "inside a work tree"
            } else {
                "not in a git repo (cwd)"
            },
            None,
        ));
    }

    // terminal — resolve the old "(?)". On Unix an interactive stdout with no usable TERM is the
    // class of box where the full-screen TUI misbehaves, so flag it. On Windows TERM is a Unix
    // concept and is normally UNSET — crossterm drives the console via the Console API regardless —
    // so an interactive Windows console is simply OK (warning there is a false positive).
    let tty = std::io::stdout().is_terminal();
    let term = std::env::var("TERM").ok().filter(|t| !t.is_empty());
    let term_usable = term.as_deref().is_some_and(|t| t != "dumb");
    // Gold-standard viability: actually enter+exit raw mode, exactly what the TUI does on launch.
    // More authoritative than the TERM heuristic — a box where this fails genuinely can't run the
    // full-screen UI, while one where it succeeds can (even with an odd TERM). Only meaningful on an
    // interactive stdout, so it's gated on `tty`.
    let raw_probe = tty.then(raw_mode_probe);
    let (status, detail, fix) = if !tty {
        (Status::Info, "non-interactive (piped/CI)".to_string(), None)
    } else if let Some(Err(e)) = &raw_probe {
        (
            Status::Warn,
            format!("interactive but raw-mode probe failed ({e}) — the full-screen TUI won't work here"),
            Some("use a different terminal emulator; ensure stdin+stdout are a real tty and TERM is set"),
        )
    } else if cfg!(windows) {
        (
            Status::Ok,
            "interactive (Windows console, raw-mode OK)".to_string(),
            None,
        )
    } else if let Some(term) = term.as_deref().filter(|_| term_usable) {
        (
            Status::Ok,
            format!("interactive ({term}, raw-mode OK)"),
            None,
        )
    } else {
        (
            Status::Warn,
            format!(
                "interactive but TERM={} — the TUI may not render correctly",
                term.as_deref().unwrap_or("(unset)")
            ),
            Some("export TERM=xterm-256color (add it to your shell profile)"),
        )
    };
    out.push(check(status, "terminal", detail, fix));

    // WSL: surface it explicitly — it's the platform behind most "hangs / won't open" reports, and
    // knowing it's WSL focuses the fix (TERM, a responsive keyring, PATH'd Windows .cmd shims).
    if is_wsl() {
        out.push(check(Status::Info, "platform", "WSL detected", None));
    }
    out
}

/// Enter then exit raw mode — the exact terminal capability the full-screen TUI needs. Returns the
/// error string if entering fails (a box that can't support the UI). Always attempts to restore
/// cooked mode so `forge doctor` never leaves the terminal in raw mode.
fn raw_mode_probe() -> Result<(), String> {
    use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
    enable_raw_mode().map_err(|e| e.to_string())?;
    disable_raw_mode().map_err(|e| e.to_string())
}

/// Best-effort WSL detection: the kernel release string carries "microsoft" under WSL1/2.
fn is_wsl() -> bool {
    std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .map(|s| s.to_lowercase().contains("microsoft"))
        .unwrap_or(false)
}

fn binary_on_path(bin: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|d| d.join(bin).is_file()))
        .unwrap_or(false)
}
