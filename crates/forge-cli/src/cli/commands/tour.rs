//! `forge tour` — a 2-minute guided introduction printed to the terminal, with an optional
//! `--demo` that runs one REAL agent turn offline (the mock provider) in a scratch directory,
//! against an isolated store, so nothing touches the user's projects or session history.

use anyhow::{Context, Result};

const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const CYAN: &str = "\x1b[36m";
const RESET: &str = "\x1b[0m";

struct Stop {
    title: &'static str,
    body: &'static str,
    try_cmd: &'static str,
}

const STOPS: &[Stop] = &[
    Stop {
        title: "Chat — the core loop",
        body: "One binary, a full coding agent: it reads, edits, runs commands, and tracks tasks. \
               Every action passes a permission gate you control (SHIFT+TAB cycles the temper: \
               Survey → Guarded → Smith → Unfettered).",
        try_cmd: "forge chat",
    },
    Stop {
        title: "The Mesh — many models, one router",
        body: "Forge routes each task across every model you have (subscriptions, API keys, free \
               tiers, local ollama) by difficulty, cost, health, and live quota — with automatic \
               failover. See exactly why a model was picked:",
        try_cmd: "forge mesh \"refactor the auth module\"",
    },
    Stop {
        title: "Provenance — every change traceable",
        body: "Sessions persist. Replay any conversation, diff two runs, see which model wrote a \
               line and what it cost:",
        try_cmd: "forge replay <id> · forge blame <file> <line> · /pr",
    },
    Stop {
        title: "Counterfactuals — change one variable",
        body: "Fork any past session before turn N and re-ask that one prompt on a different \
               model; the replay diff IS the effect of the change:",
        try_cmd: "forge fork <id> --turn 3 --model groq::llama --rerun",
    },
    Stop {
        title: "Autopilot — work while you sleep",
        body: "Queue big tasks during the day, drain them overnight: each runs budget-capped in \
               its own git worktree and leaves a review-ready branch (add --shadow to let free \
               models shadow-race the same tasks and improve routing for free):",
        try_cmd: "forge queue add \"migrate to sqlx\" --budget 2 && forge queue run",
    },
    Stop {
        title: "The arena — let models compete",
        body: "Race 2-3 mesh models on the same task in isolated worktrees, pick the winner, and \
               the router learns your preference (see the standings with `forge scoreboard`):",
        try_cmd: "/duel implement the rate limiter",
    },
    Stop {
        title: "Remote control — leave the desk",
        body: "Type /remote in chat: a QR code connects your phone to the live session — watch \
               the stream, approve permissions, answer questions, queue follow-ups. Installable \
               as a PWA.",
        try_cmd: "/remote",
    },
];

pub(crate) fn tour_cmd(demo: bool) -> Result<()> {
    println!("{BOLD}⚒ Forge — a self-contained coding agent with a model mesh{RESET}");
    println!("{DIM}The tour takes ~2 minutes. Nothing here needs an API key.{RESET}\n");
    for (i, stop) in STOPS.iter().enumerate() {
        println!(
            "{BOLD}{}. {}{RESET}\n   {}\n   {CYAN}▸ {}{RESET}\n",
            i + 1,
            stop.title,
            stop.body,
            stop.try_cmd
        );
    }
    if demo {
        run_demo()?;
    } else {
        println!(
            "{DIM}Run `forge tour --demo` to watch a real agent turn offline (mock model).{RESET}"
        );
    }
    println!(
        "\n{BOLD}Next steps{RESET}\n   {CYAN}▸ forge setup{RESET}  {DIM}connect providers (or just launch `forge chat` — keyless free models work out of the box){RESET}\n   {CYAN}▸ forge doctor{RESET}  {DIM}health-check keys + config{RESET}\n   {CYAN}▸ /help{RESET}        {DIM}every command, in-chat{RESET}"
    );
    Ok(())
}

/// One real turn through the full agent loop — mock provider, scratch cwd, isolated store — so
/// the user sees routing, a tool call, and the final answer without keys or side effects.
fn run_demo() -> Result<()> {
    println!("{BOLD}── live demo: one offline agent turn ──{RESET}");
    let dir = std::env::temp_dir().join(format!("forge-tour-{}", std::process::id()));
    std::fs::create_dir_all(&dir).context("create tour scratch dir")?;
    // Seed a tiny project so the demo's read_file lands on something real.
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"tour-demo\"\nversion = \"0.1.0\"\n",
    )
    .context("seed tour scratch project")?;
    let exe = std::env::current_exe().context("locate forge binary")?;
    let status = std::process::Command::new(exe)
        .args(["run", "read the project and summarize it", "--mock"])
        .current_dir(&dir)
        .env("FORGE_DB", dir.join("tour.db"))
        .status()
        .context("spawn demo turn")?;
    // Best-effort cleanup; the demo dir holds only the scratch DB + any mock artifacts.
    let _ = std::fs::remove_dir_all(&dir);
    if !status.success() {
        println!("{DIM}(demo turn exited non-zero — the tour text above still stands){RESET}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tour_prints_without_demo() {
        // The printed tour must never fail — it's the first thing a new user runs.
        assert!(tour_cmd(false).is_ok());
    }
}
