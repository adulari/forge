// Desktop-app behaviour that the webview cannot provide for itself: a system-wide Quick Composer
// chord, a single running instance, launch at login (optionally hidden in the tray) and
// recovery from a dead renderer process.
//
// Like the tray and the menu this holds no daemon state. Every action is handed to the webview
// through the same `forge://menu` vocabulary the tray already uses (`session:quick-composer`),
// so there is one code path from "the user asked for the composer" to the composer opening.
//
// Anything that can arrive before React has mounted a listener (a chord pressed during a cold
// start, a second launch carrying a deep link) is parked in `Handoff` and delivered when the
// webview calls `desktop_shell_ready`.
use std::collections::VecDeque;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, Runtime, State};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::menu;

pub const DEEP_LINK_EVENT: &str = "forge://deep-link";
const QUICK_COMPOSER_ID: &str = "session:quick-composer";
pub const AUTOSTART_FLAG: &str = "--autostarted";
pub const QUICK_COMPOSER_FLAG: &str = "--quick-composer";
const DEEP_LINK_SCHEME: &str = "forge://";
const MAX_PENDING_LINKS: usize = 8;

#[derive(Default)]
struct HandoffState {
    ready: bool,
    quick_composer: bool,
    deep_links: VecDeque<String>,
}

/// What the webview still has to act on once it can listen.
#[derive(Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Backlog {
    pub quick_composer: bool,
    pub deep_links: Vec<String>,
}

#[derive(Default)]
pub struct Handoff(Mutex<HandoffState>);

impl Handoff {
    /// `true` when the webview is listening and the event should be emitted now; otherwise the
    /// request is remembered for `mark_ready`.
    fn quick_composer(&self) -> bool {
        let mut state = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !state.ready {
            state.quick_composer = true;
        }
        state.ready
    }

    fn deep_link(&self, link: &str) -> bool {
        let mut state = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !state.ready {
            if state.deep_links.len() == MAX_PENDING_LINKS {
                state.deep_links.pop_front();
            }
            state.deep_links.push_back(link.to_string());
        }
        state.ready
    }

    fn mark_ready(&self) -> Backlog {
        let mut state = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.ready = true;
        Backlog {
            quick_composer: std::mem::take(&mut state.quick_composer),
            deep_links: state.deep_links.drain(..).collect(),
        }
    }

    /// A reload throws the listeners away, so anything arriving until the next
    /// `desktop_shell_ready` has to be parked again.
    fn mark_unready(&self) {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .ready = false;
    }
}

/// Deep links are only ever `forge://…`; anything else on a second launch's command line
/// (flags, file paths) is not the webview's business.
pub fn deep_links(args: &[String]) -> Vec<String> {
    args.iter()
        .filter(|arg| arg.starts_with(DEEP_LINK_SCHEME))
        .cloned()
        .collect()
}

pub fn wants_quick_composer(args: &[String]) -> bool {
    args.iter().any(|arg| arg == QUICK_COMPOSER_FLAG)
}

fn open_quick_composer<R: Runtime>(app: &AppHandle<R>) {
    menu::raise_main_window(app);
    if app.state::<Handoff>().quick_composer() {
        emit_menu(app, QUICK_COMPOSER_ID);
    }
}

fn emit_menu<R: Runtime>(app: &AppHandle<R>, id: &str) {
    let _ = app.emit(
        menu::MENU_EVENT,
        menu::MenuEventPayload {
            id: id.to_string(),
            decision: None,
        },
    );
}

/// Runs in the already-running process when a second copy is launched.
pub fn on_second_instance<R: Runtime>(app: &AppHandle<R>, args: &[String]) {
    menu::raise_main_window(app);
    if wants_quick_composer(args) {
        open_quick_composer(app);
    }
    for link in deep_links(args) {
        if app.state::<Handoff>().deep_link(&link) {
            let _ = app.emit(DEEP_LINK_EVENT, link);
        }
    }
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ChordError {
    /// The accelerator string is not a chord this shell understands.
    Invalid { message: String },
    /// Another application owns the chord, or the OS refused the registration.
    Unavailable { message: String },
    /// The session has no global-hotkey mechanism (native Wayland).
    Unsupported { message: String },
}

fn session_lacks_global_hotkeys() -> bool {
    cfg!(target_os = "linux")
        && std::env::var("XDG_SESSION_TYPE").is_ok_and(|kind| kind == "wayland")
        && std::env::var("GDK_BACKEND")
            .ok()
            .is_none_or(|backend| backend != "x11")
}

#[derive(Default)]
pub struct RegisteredChord(Mutex<Option<Shortcut>>);

fn parse_chord(accelerator: &str) -> Result<Shortcut, ChordError> {
    Shortcut::from_str(accelerator).map_err(|error| ChordError::Invalid {
        message: error.to_string(),
    })
}

#[tauri::command]
pub fn quick_composer_register<R: Runtime>(
    app: AppHandle<R>,
    registered: State<'_, RegisteredChord>,
    accelerator: String,
) -> Result<(), ChordError> {
    let shortcut = parse_chord(&accelerator)?;
    if session_lacks_global_hotkeys() {
        return Err(ChordError::Unsupported {
            message: format!(
                "This Wayland session has no global shortcuts. Bind a key in your compositor to `forge-desktop {QUICK_COMPOSER_FLAG}` instead."
            ),
        });
    }
    let hotkeys = app.global_shortcut();
    let mut slot = registered
        .0
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if slot.as_ref() == Some(&shortcut) && hotkeys.is_registered(shortcut) {
        return Ok(());
    }
    if hotkeys.is_registered(shortcut) {
        return Err(ChordError::Unavailable {
            message: "Another application is already using this shortcut.".to_string(),
        });
    }
    hotkeys
        .on_shortcut(shortcut, |app, _, event| {
            if event.state == ShortcutState::Pressed {
                open_quick_composer(app);
            }
        })
        .map_err(|error| ChordError::Unavailable {
            message: error.to_string(),
        })?;
    if let Some(previous) = slot.replace(shortcut) {
        let _ = hotkeys.unregister(previous);
    }
    Ok(())
}

#[tauri::command]
pub fn quick_composer_unregister<R: Runtime>(
    app: AppHandle<R>,
    registered: State<'_, RegisteredChord>,
) {
    let mut slot = registered
        .0
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(previous) = slot.take() {
        let _ = app.global_shortcut().unregister(previous);
    }
}

/// Called by the webview right after it starts listening for `forge://menu` and
/// `forge://deep-link`; returns what arrived before that.
#[tauri::command]
pub fn desktop_shell_ready(handoff: State<'_, Handoff>) -> Backlog {
    handoff.mark_ready()
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ShellPrefs {
    #[serde(default)]
    start_minimized: bool,
}

fn prefs_path<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    Some(app.path().app_config_dir().ok()?.join("desktop-shell.json"))
}

fn read_prefs<R: Runtime>(app: &AppHandle<R>) -> ShellPrefs {
    prefs_path(app)
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

/// A login launch with "Start minimized" on stays in the tray; every other launch shows the
/// window.
pub fn should_start_hidden(args: &[String], prefs_start_minimized: bool) -> bool {
    prefs_start_minimized && args.iter().any(|arg| arg == AUTOSTART_FLAG)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchSettings {
    launch_at_login: bool,
    start_minimized: bool,
}

#[tauri::command]
pub fn launch_settings<R: Runtime>(app: AppHandle<R>) -> Result<LaunchSettings, String> {
    Ok(LaunchSettings {
        launch_at_login: app.autolaunch().is_enabled().map_err(|e| e.to_string())?,
        start_minimized: read_prefs(&app).start_minimized,
    })
}

#[tauri::command]
pub fn set_launch_at_login<R: Runtime>(app: AppHandle<R>, enabled: bool) -> Result<(), String> {
    let launcher = app.autolaunch();
    if enabled {
        launcher.enable()
    } else {
        launcher.disable()
    }
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn set_start_minimized<R: Runtime>(app: AppHandle<R>, enabled: bool) -> Result<(), String> {
    let path = prefs_path(&app).ok_or_else(|| "config directory unavailable".to_string())?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    }
    let raw = serde_json::to_string(&ShellPrefs {
        start_minimized: enabled,
    })
    .map_err(|error| error.to_string())?;
    std::fs::write(path, raw).map_err(|error| error.to_string())
}

/// Plugin list entries, kept here so `lib.rs` only chains them.
pub fn autostart_plugin<R: Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri_plugin_autostart::init(
        tauri_plugin_autostart::MacosLauncher::LaunchAgent,
        Some(vec![AUTOSTART_FLAG]),
    )
}

pub fn single_instance_plugin<R: Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri_plugin_single_instance::init(|app, args, _cwd| on_second_instance(app, &args))
}

pub fn global_shortcut_plugin<R: Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri_plugin_global_shortcut::Builder::new().build()
}

/// Called once from the setup hook, after the tray has had its chance to install.
pub fn setup<R: Runtime>(app: &AppHandle<R>, tray_installed: bool) {
    let args: Vec<String> = std::env::args().collect();
    if tray_installed && should_start_hidden(&args, read_prefs(app).start_minimized) {
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.hide();
        }
    }
    let state = app.state::<Handoff>();
    for link in deep_links(&args) {
        state.deep_link(&link);
    }
    if wants_quick_composer(&args) {
        state.quick_composer();
        menu::raise_main_window(app);
    }
    #[cfg(target_os = "linux")]
    watch_renderer(app);
}

/// Reloads allowed per window before the renderer is declared crash-looping.
const MAX_RENDERER_RELOADS: usize = 3;
const RENDERER_RELOAD_WINDOW: Duration = Duration::from_secs(60);

#[derive(Debug, PartialEq, Eq)]
pub enum CrashAction {
    Reload,
    /// Reloading has not helped; show the labelled crash screen and stop retrying.
    GiveUp,
}

#[derive(Default)]
pub struct CrashLoopGuard(Mutex<Vec<Instant>>);

impl CrashLoopGuard {
    pub fn record(&self, now: Instant) -> CrashAction {
        let mut crashes = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        crashes.retain(|at| now.duration_since(*at) < RENDERER_RELOAD_WINDOW);
        crashes.push(now);
        if crashes.len() > MAX_RENDERER_RELOADS {
            CrashAction::GiveUp
        } else {
            CrashAction::Reload
        }
    }
}

/// WebKitGTK reports a dead web process through a signal and leaves a blank view behind. Reload
/// it, and when it keeps dying load the app with `?native_crash=loop` so the webview's crash
/// screen (ErrorBoundary) can say so instead of the window staying white.
#[cfg(target_os = "linux")]
fn watch_renderer<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let handle = app.clone();
    let _ = window.with_webview(move |webview| {
        use webkit2gtk::WebViewExt;
        let guard = std::sync::Arc::new(CrashLoopGuard::default());
        webview
            .inner()
            .connect_web_process_terminated(move |view, _reason| {
                handle.state::<Handoff>().mark_unready();
                match guard.record(Instant::now()) {
                    CrashAction::Reload => view.reload(),
                    CrashAction::GiveUp => {
                        if let Some(uri) = view.uri() {
                            let looped = if uri.contains('?') {
                                format!("{uri}&native_crash=loop")
                            } else {
                                format!("{uri}?native_crash=loop")
                            };
                            view.load_uri(&looped);
                        }
                    }
                }
            });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    #[test]
    fn chord_pressed_before_the_webview_listens_is_delivered_once_on_ready() {
        let handoff = Handoff::default();
        assert!(!handoff.quick_composer());
        assert!(!handoff.quick_composer());
        assert_eq!(
            handoff.mark_ready(),
            Backlog {
                quick_composer: true,
                deep_links: vec![]
            }
        );
        assert_eq!(handoff.mark_ready(), Backlog::default());
        assert!(handoff.quick_composer());
    }

    #[test]
    fn deep_links_queue_in_order_and_cap_at_the_newest() {
        let handoff = Handoff::default();
        for index in 0..(MAX_PENDING_LINKS + 2) {
            assert!(!handoff.deep_link(&format!("forge://session/{index}")));
        }
        let backlog = handoff.mark_ready();
        assert_eq!(backlog.deep_links.len(), MAX_PENDING_LINKS);
        assert_eq!(backlog.deep_links[0], "forge://session/2");
        assert!(handoff.deep_link("forge://new"));
    }

    #[test]
    fn a_reload_parks_events_again_until_the_next_ready() {
        let handoff = Handoff::default();
        handoff.mark_ready();
        handoff.mark_unready();
        assert!(!handoff.quick_composer());
        assert!(handoff.mark_ready().quick_composer);
    }

    #[test]
    fn only_forge_urls_are_forwarded_from_a_second_launch() {
        let launch = args(&[
            "forge-desktop",
            "--quick-composer",
            "forge://session/abc",
            "/tmp/x",
        ]);
        assert_eq!(deep_links(&launch), ["forge://session/abc"]);
        assert!(wants_quick_composer(&launch));
        assert!(!wants_quick_composer(&args(&["forge-desktop"])));
    }

    #[test]
    fn minimized_start_needs_both_the_login_launch_and_the_preference() {
        let login = args(&["forge-desktop", AUTOSTART_FLAG]);
        assert!(should_start_hidden(&login, true));
        assert!(!should_start_hidden(&login, false));
        assert!(!should_start_hidden(&args(&["forge-desktop"]), true));
    }

    #[test]
    fn shell_prefs_default_when_the_file_is_old_or_empty() {
        assert_eq!(
            serde_json::from_str::<ShellPrefs>("{}").unwrap(),
            ShellPrefs::default()
        );
        assert!(
            serde_json::from_str::<ShellPrefs>(r#"{"startMinimized":true}"#)
                .unwrap()
                .start_minimized
        );
    }

    #[test]
    fn app_accelerators_parse_and_garbage_is_a_typed_invalid_error() {
        for accelerator in [
            "Alt+Space",
            "CmdOrCtrl+Shift+Space",
            "CmdOrCtrl+Alt+K",
            "Ctrl+Shift+Backslash",
        ] {
            assert!(parse_chord(accelerator).is_ok(), "{accelerator}");
        }
        let error = parse_chord("Hyper+Nonsense").unwrap_err();
        assert!(matches!(error, ChordError::Invalid { .. }));
        assert_eq!(
            serde_json::to_value(&ChordError::Unavailable {
                message: "x".into()
            })
            .unwrap(),
            serde_json::json!({"kind": "unavailable", "message": "x"})
        );
    }

    #[test]
    fn crash_loop_guard_reloads_three_times_then_gives_up_until_the_window_passes() {
        let guard = CrashLoopGuard::default();
        let start = Instant::now();
        for offset in 0..MAX_RENDERER_RELOADS as u64 {
            assert_eq!(
                guard.record(start + Duration::from_secs(offset)),
                CrashAction::Reload
            );
        }
        assert_eq!(
            guard.record(start + Duration::from_secs(4)),
            CrashAction::GiveUp
        );
        assert_eq!(
            guard.record(start + Duration::from_secs(200)),
            CrashAction::Reload
        );
    }
}
