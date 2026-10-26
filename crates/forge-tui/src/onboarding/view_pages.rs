//! What each page shows. Every page leads with what was detected, then the one thing to do.

use forge_onboarding::{ModelSource, SourceKind, SourceState, StepId};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};

use super::pages::{
    import_items, recommended_temper, search_options, SearchOpt, CREDITS, REPORTING,
    REPORTING_DETAILS, TEMPERS, THEMES,
};
use super::screen::Screen;
use super::text::{blank, heading, paragraph, truncate};
use super::view::{padded, selectable};
use super::view_models::{entry_field, DetailLine};
use super::{view_flows, view_models};
use crate::theme::glyphs::g;
use crate::theme::styles;

/// The two half-block rows of the `FORGE` wordmark.
const WORDMARK: [[&str; 5]; 2] = [
    ["█▀▀", "█▀█", "█▀█", "█▀▀", "█▀▀"],
    ["█▀ ", "█▄█", "█▀▄", "█▄█", "██▄"],
];

pub(crate) fn page(s: &Screen, width: usize, height: usize) -> (Vec<Line<'static>>, Option<usize>) {
    if s.sign_in.is_some() {
        return (view_flows::sign_in(s, width, height), None);
    }
    if s.reconnect_card {
        return (view_flows::reconnect_card(s, width), None);
    }
    match s.step() {
        StepId::InstallCli => (
            simple(s, width, "Forge installs its command for you."),
            None,
        ),
        StepId::Welcome => (welcome(s, width, height), None),
        StepId::Models => view_models::models(s, width, height),
        StepId::Behavior => (behavior(s, width), None),
        StepId::WebSearch => search(s, width),
        StepId::Benchmarks => (benchmarks(s, width), None),
        StepId::Import => import(s, width),
        StepId::Service => (service(s, width), None),
        StepId::Anywhere => (anywhere(s, width), None),
        StepId::Surfaces => (surfaces(s, width), None),
        StepId::Preferences => (prefs(s, width), None),
        StepId::Reporting => (reporting(s, width), None),
        StepId::Done => (done(s, width), None),
    }
}

fn simple(s: &Screen, width: usize, text: &str) -> Vec<Line<'static>> {
    let mut out = vec![heading(&s.session.plan[s.page].title), blank()];
    out.extend(paragraph(text, width, 0, styles::muted()));
    out
}

fn intro(title: &str, text: &str, width: usize) -> Vec<Line<'static>> {
    let mut out = vec![heading(title)];
    out.extend(paragraph(text, width, 0, styles::muted()));
    out.push(blank());
    out
}

fn status_line(ok: bool, text: String) -> Line<'static> {
    let (glyph, style) = if ok {
        (g().ok, styles::ok())
    } else {
        (g().info, styles::faint())
    };
    Line::from(vec![
        Span::styled(format!("  {glyph} "), style),
        Span::styled(text, if ok { styles::fg() } else { styles::muted() }),
    ])
}

fn welcome(s: &Screen, width: usize, height: usize) -> Vec<Line<'static>> {
    if s.batch.is_some() {
        return view_flows::batch_checklist(s, width);
    }
    let mut out = Vec::new();
    if height >= 17 && g().unicode {
        for row in WORDMARK {
            let mut spans = vec![Span::raw("  ")];
            for (i, letter) in row.iter().enumerate() {
                spans.push(Span::styled(
                    format!("{letter} "),
                    styles::heat(i).add_modifier(Modifier::BOLD),
                ));
            }
            out.push(Line::from(spans));
        }
    } else {
        out.push(Line::from(Span::styled(
            format!("{} forge", g().brand),
            styles::brand().bold(),
        )));
    }
    out.push(Line::from(Span::styled(
        "One harness for every model. Setup takes about two minutes.",
        styles::muted(),
    )));
    out.push(blank());
    out.push(heading("Found on this machine"));
    let m = &s.session.machine;
    let sources = m.model_sources();
    let ready: Vec<&ModelSource> = sources
        .iter()
        .filter(|c| c.state == SourceState::Ready)
        .collect();
    for card in ready.iter().take(4) {
        out.push(status_line(
            true,
            truncate(&card.detail_line(), width.saturating_sub(6)),
        ));
    }
    if ready.len() > 4 {
        out.push(status_line(
            true,
            format!("and {} more models", ready.len() - 4),
        ));
    }
    if ready.is_empty() {
        out.push(status_line(
            false,
            "No model is connected yet. Setup helps you pick one.".into(),
        ));
    }
    for card in sources
        .iter()
        .filter(|c| c.state == SourceState::NeedsLogin)
        .take(2)
    {
        out.push(status_line(
            false,
            truncate(&card.detail, width.saturating_sub(6)),
        ));
    }
    if m.imports.anything() {
        let mut parts = Vec::new();
        if m.imports.commands > 0 {
            parts.push(format!("{} commands", m.imports.commands));
        }
        if m.imports.skills > 0 {
            parts.push(format!("{} skills", m.imports.skills));
        }
        if m.imports.mcp_servers > 0 {
            parts.push(format!("{} MCP servers", m.imports.mcp_servers));
        }
        let text = if parts.is_empty() {
            "Settings from other tools you can bring over".to_string()
        } else {
            format!("Settings to bring over: {}", parts.join(", "))
        };
        out.push(status_line(true, text));
    }
    let svc = &m.service;
    if svc.installed && svc.running {
        out.push(status_line(true, "Background service running".into()));
    }
    if m.search.docker {
        out.push(status_line(
            true,
            "Docker found, so a local search engine is possible".into(),
        ));
    }
    out.push(blank());
    if s.session.gate().complete {
        out.extend(paragraph(
            "Everything Forge needs already works. Press f to finish now, or Enter to look through the rest.",
            width,
            0,
            styles::fg(),
        ));
    } else {
        out.extend(paragraph(
            "Press Enter and Forge walks you through the rest. Most steps are one key.",
            width,
            0,
            styles::fg(),
        ));
    }
    out
}

// ---------------------------------------------------------------- behavior

fn radio(selected: bool) -> &'static str {
    if selected {
        g().task_active
    } else {
        g().task_open
    }
}

fn choice_row(
    active_group: bool,
    selected: bool,
    label: &str,
    desc: &str,
    recommended: bool,
    width: usize,
) -> Line<'static> {
    let tag = if recommended { "  recommended" } else { "" };
    let label_w = 17;
    let desc_w = width.saturating_sub(2 + 2 + label_w + tag.len());
    let focused = active_group && selected;
    let mut spans = vec![
        Span::styled(
            format!("{} ", radio(selected)),
            if selected {
                styles::brand()
            } else {
                styles::faint()
            },
        ),
        Span::styled(
            padded(label, label_w),
            if selected {
                styles::fg().bold()
            } else {
                styles::fg()
            },
        ),
        Span::styled(padded(desc, desc_w), styles::muted()),
    ];
    if recommended {
        spans.push(Span::styled(tag.to_string(), styles::ok()));
    }
    selectable(spans, focused, width)
}

fn behavior(s: &Screen, width: usize) -> Vec<Line<'static>> {
    let mut out = vec![heading("How careful should Forge be with your files?")];
    for (i, t) in TEMPERS.iter().enumerate() {
        out.push(choice_row(
            s.ui.behavior_group == 0,
            i == s.ui.temper,
            t.label,
            t.desc,
            i == recommended_temper(),
            width,
        ));
    }
    out.push(Line::from(Span::styled(
        truncate(
            &format!("For example: {}", TEMPERS[s.ui.temper].example),
            width,
        ),
        styles::faint(),
    )));
    out.push(blank());
    out.push(heading("How should it spend your credits?"));
    for (i, c) in CREDITS.iter().enumerate() {
        out.push(choice_row(
            s.ui.behavior_group == 1,
            i == s.ui.credit,
            c.label,
            c.desc,
            i == 0,
            width,
        ));
    }
    out.push(Line::from(Span::styled(
        truncate(
            &format!("For example: {}", CREDITS[s.ui.credit].example),
            width,
        ),
        styles::faint(),
    )));
    out
}

// ---------------------------------------------------------------- search

fn search(s: &Screen, width: usize) -> (Vec<Line<'static>>, Option<usize>) {
    let m = &s.session.machine;
    let mut out = intro(
        "Web search",
        "Forge can look things up while it works. It already searches with no key; these make it better.",
        width,
    );
    let mut focus = None;
    for (i, opt) in search_options(m).iter().enumerate() {
        let (label, desc, done) = match opt {
            SearchOpt::Builtin => ("Built in", "works now, no setup", true),
            SearchOpt::Searxng => (
                "Local search",
                "your own SearXNG in Docker, no limits",
                m.search.searxng_running,
            ),
            SearchOpt::Brave => (
                "Brave Search",
                "a Brave API key, fast and consistent",
                m.search.brave_key,
            ),
        };
        let selected = i == s.cursor;
        if selected {
            focus = Some(out.len());
        }
        let mark = if done { g().ok } else { g().task_open };
        out.push(selectable(
            vec![
                Span::styled(
                    format!("{mark} "),
                    if done { styles::ok() } else { styles::faint() },
                ),
                Span::styled(
                    padded(label, 14),
                    if selected {
                        styles::fg().bold()
                    } else {
                        styles::fg()
                    },
                ),
                Span::styled(truncate(desc, width.saturating_sub(20)), styles::muted()),
            ],
            selected,
            width,
        ));
    }
    if let Some(entry) = &s.entry {
        out.push(blank());
        out.push(heading(&entry.label));
        if let Some(hint) = &entry.hint {
            out.push(Line::from(Span::styled(hint.clone(), styles::muted())));
        }
        out.push(entry_field(s, width));
    }
    (out, focus)
}

fn benchmarks(s: &Screen, width: usize) -> Vec<Line<'static>> {
    let mut out = intro(
        "Benchmarks",
        "Forge ranks models with public benchmark data from Artificial Analysis (artificialanalysis.ai). It downloads and refreshes it by itself. You need no key.",
        width,
    );
    out.push(status_line(true, "Shared benchmark data is on".into()));
    let own = s.session.machine.benchmarks.own_key;
    out.push(status_line(
        own,
        if own {
            "Using your own key".into()
        } else {
            "Your own key is optional (press k)".into()
        },
    ));
    if let Some(entry) = &s.entry {
        out.push(blank());
        out.push(heading(&entry.label));
        out.push(entry_field(s, width));
    }
    out
}

// ---------------------------------------------------------------- import

fn import(s: &Screen, width: usize) -> (Vec<Line<'static>>, Option<usize>) {
    let mut out = intro(
        "Bring your setup",
        "Forge found settings from other tools. Pick what to carry over; nothing is deleted from the other tools.",
        width,
    );
    let mut focus = None;
    for (i, item) in import_items(&s.session.machine).iter().enumerate() {
        let checked = s.ui.import_checked.get(i).copied().unwrap_or(false);
        let selected = i == s.cursor;
        if selected {
            focus = Some(out.len());
        }
        let boxed = if checked { "[x]" } else { "[ ]" };
        out.push(selectable(
            vec![
                Span::styled(
                    format!("{boxed} "),
                    if checked {
                        styles::ok()
                    } else {
                        styles::faint()
                    },
                ),
                Span::styled(
                    truncate(&item.label, width.saturating_sub(8)),
                    if selected {
                        styles::fg().bold()
                    } else {
                        styles::fg()
                    },
                ),
            ],
            selected,
            width,
        ));
        out.push(Line::from(Span::styled(
            format!("      {}", truncate(&item.detail, width.saturating_sub(8))),
            styles::faint(),
        )));
    }
    (out, focus)
}

// ---------------------------------------------------------------- service etc.

fn service(s: &Screen, width: usize) -> Vec<Line<'static>> {
    let svc = &s.session.machine.service;
    let mut out = intro(
        "Background service",
        "A small service keeps Forge running so the desktop app and your phone can reach your sessions without a terminal open.",
        width,
    );
    if svc.installed && svc.running {
        out.push(status_line(
            true,
            "The service is installed and running".into(),
        ));
        return out;
    }
    out.push(status_line(
        false,
        if svc.installed {
            "Installed but not running".into()
        } else {
            "Not installed yet".into()
        },
    ));
    out.push(blank());
    out.push(heading("Who can reach it?"));
    for (i, (label, desc)) in [
        ("This computer", "only this machine (recommended)"),
        ("Home network", "also your phone on Wi-Fi, behind a token"),
    ]
    .iter()
    .enumerate()
    {
        out.push(choice_row(
            true,
            i == s.ui.exposure,
            label,
            desc,
            i == 0,
            width,
        ));
    }
    out
}

fn anywhere(s: &Screen, width: usize) -> Vec<Line<'static>> {
    let mut out = intro(
        "Forge Anywhere",
        "Optional. Reach your sessions from your phone, from anywhere, end to end encrypted. You sign in with GitHub once.",
        width,
    );
    out.push(status_line(
        s.session.machine.anywhere.signed_in,
        if s.session.machine.anywhere.signed_in {
            "Signed in".into()
        } else {
            "Not set up. Skipping is fine.".into()
        },
    ));
    out
}

fn surfaces(s: &Screen, width: usize) -> Vec<Line<'static>> {
    let mut out = intro(
        "Desktop app",
        "Optional. A window for your sessions with the same models and settings as the terminal.",
        width,
    );
    let installed = s.session.machine.desktop.installed;
    out.push(status_line(
        installed,
        if installed {
            "Installed".into()
        } else {
            "Not installed".into()
        },
    ));
    out
}

fn prefs(s: &Screen, width: usize) -> Vec<Line<'static>> {
    let mut out = intro(
        "Notifications and theme",
        "Two quick switches. Change them any time in /config.",
        width,
    );
    let rows = [
        (
            "Notifications",
            if s.ui.notifications { "on" } else { "off" }.to_string(),
            "tell me when a long turn ends or Forge is waiting",
        ),
        (
            "Theme",
            THEMES[s.ui.theme].to_string(),
            "auto follows your terminal",
        ),
    ];
    for (i, (label, value, desc)) in rows.iter().enumerate() {
        out.push(selectable(
            vec![
                Span::styled(padded(label, 15), styles::fg().bold()),
                Span::styled(padded(value, 7), styles::ok().add_modifier(Modifier::BOLD)),
                Span::styled(truncate(desc, width.saturating_sub(26)), styles::muted()),
            ],
            i == s.ui.prefs_row,
            width,
        ));
    }
    out
}

fn reporting(s: &Screen, width: usize) -> Vec<Line<'static>> {
    let mut out = intro(
        "Error reports",
        "Forge can send scrubbed error reports so bugs get fixed. Never your prompts, code, paths or secrets.",
        width,
    );
    for (i, r) in REPORTING.iter().enumerate() {
        out.push(choice_row(
            true,
            i == s.ui.reporting,
            r.label,
            r.desc,
            i == 0,
            width,
        ));
    }
    out.push(blank());
    if s.ui.reporting_details {
        out.extend(paragraph(REPORTING_DETAILS, width, 0, styles::muted()));
    } else {
        out.push(Line::from(Span::styled(
            "Press w to see exactly what gets sent.",
            styles::faint(),
        )));
    }
    out
}

fn done(s: &Screen, width: usize) -> Vec<Line<'static>> {
    let m = &s.session.machine;
    let mut out = vec![heading("Forge is ready"), blank()];
    let ready = m.ready_sources();
    if ready.is_empty() {
        out.push(status_line(
            false,
            "No model is connected. Press b to go back and add one.".into(),
        ));
    }
    for card in ready.iter().take(4) {
        let verified = s.session.state.verification(&card.id).map(|v| v.status);
        let suffix = match verified {
            Some(forge_onboarding::VerifyStatus::Verified) => ", checked",
            _ => "",
        };
        out.push(status_line(
            true,
            truncate(
                &format!("{}{suffix}", card.detail_line()),
                width.saturating_sub(6),
            ),
        ));
    }
    if ready.len() > 4 {
        out.push(status_line(
            true,
            format!("and {} more models", ready.len() - 4),
        ));
    }
    out.push(status_line(
        true,
        format!(
            "{} · {}",
            TEMPERS[s.ui.temper].label, CREDITS[s.ui.credit].label
        ),
    ));
    if m.service.installed && m.service.running {
        out.push(status_line(true, "Background service running".into()));
    }
    out.push(blank());
    if m.project.git_repo {
        let on = s.ui.trust_folder;
        out.push(Line::from(vec![
            Span::styled(
                if on { "[x] " } else { "[ ] " },
                if on { styles::ok() } else { styles::faint() },
            ),
            Span::styled(
                truncate(
                    &format!(
                        "Trust this folder so Forge uses its project settings ({})",
                        m.project.cwd
                    ),
                    width.saturating_sub(4),
                ),
                styles::fg(),
            ),
        ]));
    }
    out.extend(paragraph(
        "Run `forge doctor` any time to check everything, or `forge setup` to come back here.",
        width,
        0,
        styles::muted(),
    ));
    out
}

// ---------------------------------------------------------------- chips

/// The Enter chip, then the page's other keys in priority order.
pub(crate) fn chips_for(s: &Screen) -> ((String, String), Vec<(String, String)>) {
    let enter = ("Enter".to_string(), s.enter_label());
    let c = |k: &str, l: &str| (k.to_string(), l.to_string());
    if s.sign_in.is_some() && s.entry.is_none() {
        return (
            c("o", "Open the page"),
            vec![c("v", "QR code"), c("Esc", "Cancel")],
        );
    }
    if s.reconnect_card && s.entry.is_none() {
        return (
            ("Enter".to_string(), "Connect now".to_string()),
            vec![c("l", "Later"), c("n", "Don't ask again")],
        );
    }
    if s.entry.is_some() {
        let mut keys = vec![c("Esc", "Cancel"), c("Ctrl+U", "Clear")];
        if s.entry.as_ref().is_some_and(|e| e.url.is_some()) {
            keys.push(c("Ctrl+O", "Open the page"));
        }
        return (enter, keys);
    }
    let keys = match s.step() {
        StepId::Welcome => {
            let mut keys = vec![c("n", "Go step by step")];
            if s.session.gate().complete {
                keys.push(c("f", "Looks good, finish"));
            }
            keys
        }
        StepId::Models => {
            let mut keys = vec![c("↑↓", "Choose"), c("Space", "Set up or check")];
            if let Some(card) = s.cards().get(s.cursor) {
                if card.key_url.is_some() {
                    keys.push(c("o", "Key page"));
                }
                if card.kind == SourceKind::ClaudeCode && card.state != SourceState::NotInstalled {
                    keys.push(c("t", "Claude token"));
                }
            }
            keys.push(c("r", "Check again"));
            keys.push(c("s", "Skip for now"));
            keys
        }
        StepId::Behavior => vec![c("↑↓", "Choose"), c("←→", "Switch list"), c("s", "Skip")],
        StepId::WebSearch | StepId::Service => vec![c("↑↓", "Choose"), c("s", "Skip")],
        StepId::Import => vec![c("Space", "Toggle"), c("↑↓", "Move"), c("s", "Skip")],
        StepId::Benchmarks => vec![c("k", "Use my own key"), c("s", "Skip")],
        StepId::Preferences => vec![c("↑↓", "Row"), c("←→", "Change"), c("s", "Skip")],
        StepId::Reporting => vec![c("↑↓", "Choose"), c("w", "What is sent"), c("s", "Skip")],
        StepId::Anywhere | StepId::Surfaces => vec![c("s", "Skip")],
        StepId::Done if s.session.machine.project.git_repo => vec![c("t", "Trust this folder")],
        _ => Vec::new(),
    };
    (enter, keys)
}
