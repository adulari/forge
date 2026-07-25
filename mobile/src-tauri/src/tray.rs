// Machined "D Tray Notif About" (docs/design/machined/Desktop.dc.html L960-988): the menu-bar
// extra — an ember mark with a needs-you badge, a dropdown carrying the waiting decision, a
// compact session list, and Open Forge / Quick Composer.
//
// The shell never talks to the daemon (serve_discovery.rs header): the webview already holds
// live fleet state, so it PUSHES a redacted summary in with `set_tray_summary` and this module
// only re-renders the menu from it. That also keeps the privacy rule in one place — the summary
// arrives already generic for Anywhere-routed/locked sessions (see `desktopNotify.ts`), so
// there is no path by which locked content can reach the tray.
use std::sync::atomic::{AtomicBool, Ordering};

use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Runtime};

use crate::menu;

pub const TRAY_ID: &str = "forge";

/// Whether the menu-bar extra actually came up. `lib.rs` only keeps the app alive past its last
/// window when it did — parking Forge in a tray that does not exist would leave a running
/// process with no surface at all.
static INSTALLED: AtomicBool = AtomicBool::new(false);

pub fn is_installed() -> bool {
    INSTALLED.load(Ordering::Relaxed)
}

pub const TRAY_OPEN: &str = "tray:open";
pub const TRAY_QUICK_COMPOSER: &str = "tray:quick-composer";
pub const TRAY_DECISION_ALLOW: &str = "tray:decision:allow";
pub const TRAY_DECISION_DENY: &str = "tray:decision:deny";
/// Session rows use `tray:session:<id>`; `desktopMenu.ts` splits the id back off the prefix.
pub const TRAY_SESSION_PREFIX: &str = "tray:session:";

/// The dropdown is a glance surface, not a session browser — past ~6 rows it stops being
/// readable at a glance and the app window is the better answer.
const MAX_SESSIONS: usize = 6;

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TraySummary {
    #[serde(default)]
    pub busy: u32,
    #[serde(default)]
    pub waiting: u32,
    pub cost_usd: Option<f64>,
    pub decision: Option<TrayDecision>,
    #[serde(default)]
    pub sessions: Vec<TraySession>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrayDecision {
    /// Already redacted webview-side when the session is Anywhere-routed or the app is locked.
    pub title: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TraySession {
    pub id: String,
    pub title: String,
    /// `"waiting" | "busy" | "idle"`. Free-form on purpose: an unknown value degrades to the
    /// idle glyph instead of failing the whole push.
    pub state: String,
    /// Trailing mono caption in the design (`4m`, `atlas`).
    pub meta: Option<String>,
}

/// Menu rows cannot carry colour, so the design's status dot becomes a leading glyph.
fn state_glyph(state: &str) -> &'static str {
    match state {
        "waiting" => "!",
        "busy" => "●",
        _ => "○",
    }
}

/// Design's header line: `2 forging · $2.27`.
fn header_text(summary: &TraySummary) -> String {
    let mut parts: Vec<String> = Vec::new();
    if summary.waiting > 0 {
        parts.push(format!("{} waiting", summary.waiting));
    }
    if summary.busy > 0 {
        parts.push(format!("{} forging", summary.busy));
    }
    if parts.is_empty() {
        parts.push("idle".to_string());
    }
    if let Some(cost) = summary.cost_usd {
        parts.push(format!("${cost:.2}"));
    }
    parts.join(" · ")
}

fn apply<R: Runtime>(app: &AppHandle<R>, summary: &TraySummary) -> tauri::Result<()> {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return Ok(());
    };

    let header = header_text(summary);
    let mut builder = MenuBuilder::new(app).item(
        // Disabled row — the design's status line, not an action.
        &MenuItemBuilder::with_id("tray:header", format!("Forge — {header}"))
            .enabled(false)
            .build(app)?,
    );

    if let Some(decision) = &summary.decision {
        builder = builder
            .separator()
            .item(
                &MenuItemBuilder::with_id("tray:decision", &decision.title)
                    .enabled(false)
                    .build(app)?,
            )
            .item(&MenuItemBuilder::with_id(TRAY_DECISION_ALLOW, "Allow").build(app)?)
            .item(&MenuItemBuilder::with_id(TRAY_DECISION_DENY, "Deny").build(app)?);
    }

    if !summary.sessions.is_empty() {
        builder = builder.separator();
        for session in summary.sessions.iter().take(MAX_SESSIONS) {
            let label = match &session.meta {
                Some(meta) => format!(
                    "{} {}  ·  {meta}",
                    state_glyph(&session.state),
                    session.title
                ),
                None => format!("{} {}", state_glyph(&session.state), session.title),
            };
            builder = builder.item(
                &MenuItemBuilder::with_id(format!("{TRAY_SESSION_PREFIX}{}", session.id), label)
                    .build(app)?,
            );
        }
    }

    let menu = builder
        .separator()
        .item(&MenuItemBuilder::with_id(TRAY_OPEN, "Open Forge").build(app)?)
        .item(&MenuItemBuilder::with_id(TRAY_QUICK_COMPOSER, "Quick Composer").build(app)?)
        .separator()
        // Windows and Linux never get the application menu bar (`lib.rs` installs it on macOS
        // only), so without this row the About panel — and with it Documentation / Report an
        // Issue / Acknowledgements — is unreachable on two of the three desktop targets.
        .item(&MenuItemBuilder::with_id(menu::APP_ABOUT, "About Forge").build(app)?)
        // Deliberately not `PredefinedMenuItem::quit`: see the identical item in `menu.rs`.
        // Closing the window no longer exits, so this is the exit, and it has to be the one
        // `lib.rs`'s prevent-exit guard lets through.
        .item(&MenuItemBuilder::with_id(menu::APP_QUIT, "Quit Forge").build(app)?)
        .build()?;

    tray.set_menu(Some(menu))?;
    tray.set_tooltip(Some(format!("Forge — {header}")))?;
    // macOS/Linux render this beside the icon — the design's needs-you badge. Cleared (rather
    // than set to "0") when nothing is waiting so the menu bar stays quiet.
    tray.set_title(if summary.waiting > 0 {
        Some(summary.waiting.to_string())
    } else {
        None
    })?;
    Ok(())
}

/// Creates the tray at startup with an empty summary; the webview fills it in once fleet state
/// has loaded.
pub fn install<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("Forge")
        // Tray menu activations do NOT reach `App::on_menu_event` (that handler only sees the
        // application/window menu), so the tray needs its own hook into the same dispatcher.
        .on_menu_event(|app, event| menu::handle_event(app, event.id().as_ref()));
    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }
    builder.build(app)?;
    INSTALLED.store(true, Ordering::Relaxed);
    apply(app, &TraySummary::default())
}

/// Webview → shell push. Errors are stringified rather than swallowed so a malformed summary
/// shows up in the caller's promise instead of silently leaving a stale tray.
#[tauri::command]
pub fn set_tray_summary<R: Runtime>(app: AppHandle<R>, summary: TraySummary) -> Result<(), String> {
    apply(&app, &summary).map_err(|e| e.to_string())
}
