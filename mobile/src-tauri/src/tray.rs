// Machined "D Tray Notif About" (docs/design/machined/Desktop.dc.html L960-988): the menu-bar
// extra — an ember mark with a needs-you badge, a dropdown carrying the waiting decision, a
// compact session list, and Open Forge / Quick Composer.
//
// The shell never talks to the daemon (serve_discovery.rs header): the webview already holds
// live fleet state, so it PUSHES a redacted summary in with `set_tray_summary` and this module
// only re-renders the menu from it. That also keeps the privacy rule in one place — the summary
// arrives already generic for Anywhere-routed/locked sessions (see `desktopNotify.ts`), so
// there is no path by which locked content can reach the tray.
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

use tauri::image::Image;
use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, Runtime, Theme};

use crate::menu;

pub const TRAY_ID: &str = "forge";

const GLYPH_ON_LIGHT: &[u8] = include_bytes!("../icons/tray-on-light.png");
const GLYPH_ON_DARK: &[u8] = include_bytes!("../icons/tray-on-dark.png");

/// The mark is a monochrome glyph so it reads on both panel colours: dark ink for a light
/// system theme, light ink for a dark one. macOS tints a template image itself, so it only ever
/// gets the dark glyph (see `install`).
fn glyph_for(theme: Theme) -> &'static [u8] {
    match theme {
        Theme::Dark => GLYPH_ON_DARK,
        _ => GLYPH_ON_LIGHT,
    }
}

/// Re-skins the tray glyph when the system theme flips. Cheap and idempotent, so the window's
/// `ThemeChanged` event can call it unconditionally.
pub fn apply_theme<R: Runtime>(app: &AppHandle<R>, theme: Theme) {
    if cfg!(target_os = "macos") {
        return;
    }
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    if let Ok(icon) = Image::from_bytes(glyph_for(theme)) {
        let _ = tray.set_icon(Some(icon));
    }
}

/// Whether the menu-bar extra actually came up. `lib.rs` only keeps the app alive past its last
/// window when it did — parking Forge in a tray that does not exist would leave a running
/// process with no surface at all.
static INSTALLED: AtomicBool = AtomicBool::new(false);

pub fn is_installed() -> bool {
    INSTALLED.load(Ordering::Relaxed)
}

pub const TRAY_OPEN: &str = "tray:open";
pub const TRAY_QUICK_COMPOSER: &str = "tray:quick-composer";
/// A session in the Home scope (no project, no worktree); the webview creates it.
pub const TRAY_HOME_SESSION: &str = "tray:home-session";
/// Decision rows. Their menu ids carry the generation of the summary they were built from
/// (`tray:decision:allow#7`), so a click on a row from a menu that has since been replaced is
/// recognised as stale instead of being applied to whatever is waiting now.
pub const TRAY_DECISION_ALLOW: &str = "tray:decision:allow";
pub const TRAY_DECISION_DENY: &str = "tray:decision:deny";
/// Opens the waiting session. The only row for questions and plan approvals, which a yes/no
/// cannot answer, and for redacted (Anywhere) prompts, which nobody should approve unread.
pub const TRAY_DECISION_REVIEW: &str = "tray:decision:review";
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
    /// Which stored server the session belongs to (an id, never the token-bearing base URL).
    pub server_id: Option<String>,
    pub session_id: Option<String>,
    /// The snapshot's `prompt_seq` — the daemon rejects an answer carrying any other value.
    /// Present only when the webview decided Allow/Deny may be offered (a permission prompt,
    /// readable, on a known session).
    pub prompt_seq: Option<u64>,
}

/// What a decision row's click hands back to the webview.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionTarget {
    pub server_id: String,
    pub session_id: String,
    pub prompt_seq: Option<u64>,
}

impl TrayDecision {
    fn target(&self) -> Option<DecisionTarget> {
        Some(DecisionTarget {
            server_id: self.server_id.clone()?,
            session_id: self.session_id.clone()?,
            prompt_seq: self.prompt_seq,
        })
    }

    /// Allow/Deny only for a fully identified permission prompt.
    fn answerable(&self) -> bool {
        self.prompt_seq.is_some() && self.target().is_some()
    }
}

/// Bumped on every re-render so row ids from a replaced menu no longer match.
static GENERATION: AtomicU64 = AtomicU64::new(0);
static CURRENT: Mutex<Option<(u64, DecisionTarget)>> = Mutex::new(None);

/// `(base id, label)` for each actionable decision row, in menu order.
fn decision_rows(decision: &TrayDecision) -> Vec<(&'static str, &'static str)> {
    let mut rows = Vec::new();
    if decision.answerable() {
        rows.push((TRAY_DECISION_ALLOW, "Allow"));
        rows.push((TRAY_DECISION_DENY, "Deny"));
    }
    rows.push((TRAY_DECISION_REVIEW, "Review Decision…"));
    rows
}

fn row_id(base: &str, generation: u64) -> String {
    format!("{base}#{generation}")
}

/// Splits a clicked row id into its base id and generation; `None` for non-decision ids.
fn parse_row_id(id: &str) -> Option<(&str, u64)> {
    let (base, generation) = id.split_once('#')?;
    if ![
        TRAY_DECISION_ALLOW,
        TRAY_DECISION_DENY,
        TRAY_DECISION_REVIEW,
    ]
    .contains(&base)
    {
        return None;
    }
    Some((base, generation.parse().ok()?))
}

/// The payload a decision-row click emits: the target the row was built for, or `None` when
/// the menu it came from has been replaced since.
fn resolve_click(
    base: &str,
    generation: u64,
    current: Option<&(u64, DecisionTarget)>,
) -> menu::MenuEventPayload {
    let decision = current
        .filter(|(current_generation, _)| *current_generation == generation)
        .map(|(_, target)| target.clone());
    menu::MenuEventPayload {
        id: base.to_string(),
        decision,
    }
}

fn handle_tray_event<R: Runtime>(app: &AppHandle<R>, id: &str) {
    let Some((base, generation)) = parse_row_id(id) else {
        menu::handle_event(app, id);
        return;
    };
    let payload = {
        let current = CURRENT.lock().ok();
        resolve_click(
            base,
            generation,
            current.as_ref().and_then(|slot| slot.as_ref()),
        )
    };
    // Answering from the tray is the point of Allow/Deny, so those leave the window where it is;
    // the webview raises it itself (`tray_open_main`) when it cannot act. Review always opens.
    if base == TRAY_DECISION_REVIEW || payload.decision.is_none() {
        menu::raise_main_window(app);
    }
    let _ = app.emit(menu::MENU_EVENT, payload);
}

/// Lets the webview bring the window forward when a tray answer could not be applied (app
/// locked, prompt changed) and the user has to look at it.
#[tauri::command]
pub fn tray_open_main<R: Runtime>(app: AppHandle<R>) {
    menu::raise_main_window(&app);
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
    let generation = GENERATION.fetch_add(1, Ordering::Relaxed) + 1;
    if let Ok(mut current) = CURRENT.lock() {
        *current = summary
            .decision
            .as_ref()
            .and_then(TrayDecision::target)
            .map(|target| (generation, target));
    }
    let mut builder = MenuBuilder::new(app).item(
        // Disabled row — the design's status line, not an action.
        &MenuItemBuilder::with_id("tray:header", format!("Forge — {header}"))
            .enabled(false)
            .build(app)?,
    );

    if let Some(decision) = &summary.decision {
        builder = builder.separator().item(
            &MenuItemBuilder::with_id("tray:decision", &decision.title)
                .enabled(false)
                .build(app)?,
        );
        for (base, label) in decision_rows(decision) {
            builder = builder
                .item(&MenuItemBuilder::with_id(row_id(base, generation), label).build(app)?);
        }
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
        .item(&MenuItemBuilder::with_id(TRAY_HOME_SESSION, "New Home session").build(app)?)
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
        .on_menu_event(|app, event| handle_tray_event(app, event.id().as_ref()));
    let theme = app
        .get_webview_window("main")
        .and_then(|window| window.theme().ok())
        .unwrap_or(Theme::Dark);
    let glyph = if cfg!(target_os = "macos") {
        GLYPH_ON_LIGHT
    } else {
        glyph_for(theme)
    };
    if let Ok(icon) = Image::from_bytes(glyph) {
        builder = builder.icon(icon);
    } else if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }
    #[cfg(target_os = "macos")]
    {
        builder = builder.icon_as_template(true);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(json: &str) -> TraySummary {
        serde_json::from_str(json).expect("webview summary parses")
    }

    #[test]
    fn glyph_ink_follows_the_system_theme_and_both_assets_decode() {
        assert_eq!(glyph_for(Theme::Dark), GLYPH_ON_DARK);
        assert_eq!(glyph_for(Theme::Light), GLYPH_ON_LIGHT);
        assert_ne!(GLYPH_ON_DARK, GLYPH_ON_LIGHT);
        for bytes in [GLYPH_ON_DARK, GLYPH_ON_LIGHT] {
            let image = Image::from_bytes(bytes).expect("tray glyph is a PNG");
            assert_eq!((image.width(), image.height()), (44, 44));
        }
    }

    #[test]
    fn allow_and_deny_only_for_an_identified_permission_prompt() {
        let permission = summary(
            r#"{"decision":{"title":"Run tests","serverId":"s1","sessionId":"abc","promptSeq":4}}"#,
        );
        let ids: Vec<_> = decision_rows(permission.decision.as_ref().unwrap())
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(
            ids,
            [
                TRAY_DECISION_ALLOW,
                TRAY_DECISION_DENY,
                TRAY_DECISION_REVIEW
            ]
        );

        // A question / plan approval / redacted prompt arrives without a promptSeq.
        let question =
            summary(r#"{"decision":{"title":"Which db?","serverId":"s1","sessionId":"abc"}}"#);
        let ids: Vec<_> = decision_rows(question.decision.as_ref().unwrap())
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(ids, [TRAY_DECISION_REVIEW]);

        // An old webview that only sends a title never gets Allow/Deny either.
        let legacy = summary(r#"{"decision":{"title":"x"}}"#);
        assert_eq!(decision_rows(legacy.decision.as_ref().unwrap()).len(), 1);
    }

    #[test]
    fn row_ids_round_trip_and_reject_foreign_ids() {
        assert_eq!(
            parse_row_id(&row_id(TRAY_DECISION_DENY, 12)),
            Some((TRAY_DECISION_DENY, 12))
        );
        assert_eq!(parse_row_id("tray:open"), None);
        assert_eq!(parse_row_id("tray:session:abc#3"), None);
        assert_eq!(parse_row_id("tray:decision:allow#x"), None);
    }

    #[test]
    fn click_payload_carries_the_target_of_its_own_menu_only() {
        let target = DecisionTarget {
            server_id: "s1".into(),
            session_id: "abc".into(),
            prompt_seq: Some(4),
        };
        let current = (7, target.clone());

        let payload = resolve_click(TRAY_DECISION_ALLOW, 7, Some(&current));
        assert_eq!(payload.decision, Some(target));
        assert_eq!(
            serde_json::to_value(&payload).unwrap(),
            serde_json::json!({
                "id": "tray:decision:allow",
                "decision": {"serverId": "s1", "sessionId": "abc", "promptSeq": 4}
            })
        );

        // A row from a replaced menu (older generation) or with nothing waiting is stale.
        assert_eq!(
            resolve_click(TRAY_DECISION_ALLOW, 6, Some(&current)).decision,
            None
        );
        let stale = resolve_click(TRAY_DECISION_DENY, 7, None);
        assert_eq!(
            serde_json::to_value(&stale).unwrap(),
            serde_json::json!({"id": "tray:decision:deny"})
        );
    }
}
