// Machined "D Menus" (docs/design/machined/INVENTORY.md § 08 Native, Desktop.dc.html L901-955):
// the full application menu bar — Forge / File / Edit / Session / View / Window / Help.
//
// The shell holds NO daemon state (see serve_discovery.rs's header) so no menu item can act on
// its own: every non-standard item carries a stable string id, and activating it emits
// `MENU_EVENT` to the webview, where `src/lib/desktopMenu.ts` maps the id onto the real RN
// action (router navigation, command palette, session socket). The two exceptions are handled
// here because they are process- or window-level facts the webview cannot know or do: opening
// the About window, quitting, and raising/focusing the main window for tray actions that may
// fire while it is hidden.
//
// This module is compiled on every platform (so `cargo build` type-checks the whole tree on
// Linux/CI) but only *installed* on macOS — see `lib.rs`. Windows and Linux hide their native
// decorations and render the Machined chrome in the webview, so attaching a native menu bar
// there would draw a second, un-themed strip above it; those platforms reach the same actions
// through the in-webview hotkey registry (`src/lib/shortcuts/`) and the tray.
use tauri::menu::{Menu, MenuItemBuilder, SubmenuBuilder};
use tauri::{AppHandle, Emitter, Manager, Runtime};

#[cfg(target_os = "macos")]
use tauri::menu::{MenuItem, MenuItemKind};

use crate::about;

/// Webview event carrying an activated menu/tray item id. Listened for in `desktopMenu.ts`.
pub const MENU_EVENT: &str = "forge://menu";

#[derive(Clone, serde::Serialize)]
pub struct MenuEventPayload {
    pub id: String,
}

// Item ids. Kept as consts so the Rust menu tree and the TS switch can be diffed by grepping
// one string; `desktopMenu.ts` mirrors this list verbatim.
pub const APP_ABOUT: &str = "app:about";
pub const APP_CHECK_UPDATES: &str = "app:check-updates";
pub const APP_SETTINGS: &str = "app:settings";
/// Never reaches `desktopMenu.ts` — `handle_event` swallows it (see the quit item below).
pub const APP_QUIT: &str = "app:quit";

pub const SESSION_NEW: &str = "session:new";
pub const SESSION_QUICK_COMPOSER: &str = "session:quick-composer";
pub const SESSION_SEARCH: &str = "session:search";
pub const SESSION_APPROVE: &str = "session:approve";
pub const SESSION_INTERRUPT: &str = "session:interrupt";
pub const SESSION_FORK: &str = "session:fork";
pub const SESSION_CHECKPOINT: &str = "session:checkpoint";
pub const SESSION_HANDOFF: &str = "session:handoff";
pub const SESSION_SHARE_REPLAY: &str = "session:share-replay";
pub const SESSION_ARCHIVE: &str = "session:archive";

pub const VIEW_SIDEBAR: &str = "view:sidebar";
pub const VIEW_SPLIT_PANE: &str = "view:split-pane";
pub const VIEW_TERMINAL: &str = "view:terminal";
pub const VIEW_USAGE: &str = "view:usage";
pub const VIEW_NOTES: &str = "view:notes";
pub const VIEW_GIT_REVIEW: &str = "view:git-review";
pub const VIEW_BROWSER_PREVIEW: &str = "view:browser-preview";
pub const VIEW_APPEARANCE_LIGHT: &str = "view:appearance:light";
pub const VIEW_APPEARANCE_DARK: &str = "view:appearance:dark";
pub const VIEW_APPEARANCE_SYSTEM: &str = "view:appearance:system";

pub const HELP_DOCS: &str = "help:docs";
pub const HELP_ISSUE: &str = "help:issue";
pub const HELP_ACKNOWLEDGEMENTS: &str = "help:acknowledgements";

/// Debug-only webview reload (pre-existing behaviour, kept).
pub const DEV_RELOAD: &str = "dev:reload";

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub struct MenuAccelerator {
    id: String,
    accelerator: Option<String>,
}

#[cfg(target_os = "macos")]
fn mutable_accelerator(id: &str) -> bool {
    matches!(
        id,
        SESSION_NEW
            | SESSION_QUICK_COMPOSER
            | SESSION_SEARCH
            | SESSION_INTERRUPT
            | VIEW_SIDEBAR
            | VIEW_SPLIT_PANE
            | VIEW_TERMINAL
            | VIEW_USAGE
            | VIEW_GIT_REVIEW
    )
}

#[cfg(target_os = "macos")]
fn find_menu_items<R: Runtime>(items: Vec<MenuItemKind<R>>, id: &str) -> Vec<MenuItem<R>> {
    let mut matches = Vec::new();
    for item in items {
        match item {
            MenuItemKind::MenuItem(item) if item.id().as_ref() == id => matches.push(item),
            MenuItemKind::Submenu(submenu) => {
                if let Ok(children) = submenu.items() {
                    matches.extend(find_menu_items(children, id));
                }
            }
            _ => {}
        }
    }
    matches
}

/// Keeps macOS menu key equivalents aligned with the persisted webview shortcut
/// preferences. Only the closed list above is mutable: standard Edit, Quit, and
/// window-management accelerators remain native platform behavior.
#[tauri::command]
pub fn set_menu_accelerators<R: Runtime>(
    app: AppHandle<R>,
    accelerators: Vec<MenuAccelerator>,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let menu = app
            .menu()
            .ok_or_else(|| "application menu is not installed".to_string())?;
        let mut resolved = Vec::with_capacity(accelerators.len());
        for update in accelerators {
            if !mutable_accelerator(&update.id) {
                return Err(format!("menu accelerator is not mutable: {}", update.id));
            }
            let items =
                find_menu_items(menu.items().map_err(|error| error.to_string())?, &update.id);
            if items.is_empty() {
                return Err(format!("menu item is missing: {}", update.id));
            }
            resolved.push((items, update.accelerator));
        }

        // Clear the closed set first so swapping two bindings never collides with
        // the key equivalent that the other item still owns. SESSION_NEW occurs
        // in File and Session with one shared event id; clear both duplicates.
        for (items, _) in &resolved {
            for item in items {
                item.set_accelerator(None::<&str>)
                    .map_err(|error| error.to_string())?;
            }
        }
        for (items, accelerator) in resolved {
            // Keep a duplicated command's key equivalent on its final menu
            // occurrence. For New Session that is the canonical Session menu,
            // matching the initial menu construction above.
            if let Some(item) = items.last() {
                item.set_accelerator(accelerator.as_deref())
                    .map_err(|error| error.to_string())?;
            }
        }
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, accelerators);
        Ok(())
    }
}

/// Accelerators are written with `keyboard-types` code names (`Period`, `Backslash`, `Comma`)
/// rather than raw punctuation — the raw forms are accepted inconsistently by the accelerator
/// parser, and a rejected accelerator is a hard error at menu-build time.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn install<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let item =
        |id: &str, text: &str, accel: Option<&str>| -> tauri::Result<tauri::menu::MenuItem<R>> {
            let mut builder = MenuItemBuilder::with_id(id, text);
            if let Some(accel) = accel {
                builder = builder.accelerator(accel);
            }
            builder.build(app)
        };

    // ---- Forge ------------------------------------------------------------------------
    let forge = SubmenuBuilder::new(app, "Forge")
        .item(&item(APP_ABOUT, "About Forge", None)?)
        .item(&item(APP_CHECK_UPDATES, "Check for Updates…", None)?)
        .separator()
        .item(&item(APP_SETTINGS, "Settings…", Some("CmdOrCtrl+Comma"))?);

    #[cfg(debug_assertions)]
    let forge = forge
        .separator()
        .item(&item(DEV_RELOAD, "Reload", Some("CmdOrCtrl+R"))?);

    // A custom item rather than `PredefinedMenuItem::quit`: the predefined one terminates
    // through the platform (`NSApp terminate:`, `PostQuitMessage`), which never produces the
    // `ExitRequested { code: Some(_) }` that `lib.rs`'s prevent-exit guard lets through. Going
    // via `AppHandle::exit` keeps ⌘Q a guaranteed exit on every platform now that closing the
    // window no longer is one. Label and accelerator match the predefined item exactly.
    let forge = forge
        .separator()
        .hide()
        .hide_others()
        .show_all()
        .separator()
        .item(&item(APP_QUIT, "Quit Forge", Some("CmdOrCtrl+Q"))?)
        .build()?;

    // ---- File -------------------------------------------------------------------------
    // "New Session…" appears here too because that is where macOS users look for it, but the
    // ⌘N accelerator is registered once, on the Session menu the design specifies — a duplicate
    // key equivalent would render twice and only ever dispatch to the first match.
    let file = SubmenuBuilder::new(app, "File")
        .item(&item(SESSION_NEW, "New Session…", None)?)
        .separator()
        .close_window()
        .build()?;

    // ---- Edit -------------------------------------------------------------------------
    // Not in the design frame, but mandatory: WKWebView gets ⌘X/⌘C/⌘V/⌘A from the native menu's
    // key equivalents, so dropping this submenu silently breaks clipboard support in every text
    // field in the app.
    let edit = SubmenuBuilder::new(app, "Edit")
        .undo()
        .redo()
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .build()?;

    // ---- Session ----------------------------------------------------------------------
    let session = SubmenuBuilder::new(app, "Session")
        .item(&item(SESSION_NEW, "New Session…", Some("CmdOrCtrl+N"))?)
        .item(&item(
            SESSION_QUICK_COMPOSER,
            "Quick Composer",
            Some("Alt+Space"),
        )?)
        .item(&item(
            SESSION_SEARCH,
            "Search Sessions…",
            Some("CmdOrCtrl+P"),
        )?)
        .separator()
        .item(&item(
            SESSION_APPROVE,
            "Approve Waiting Decision",
            Some("CmdOrCtrl+Enter"),
        )?)
        .item(&item(
            SESSION_INTERRUPT,
            "Interrupt Session",
            Some("CmdOrCtrl+Period"),
        )?)
        .item(&item(SESSION_FORK, "Fork From Here…", None)?)
        .item(&item(
            SESSION_CHECKPOINT,
            "Create Checkpoint",
            Some("CmdOrCtrl+S"),
        )?)
        .separator()
        .item(&item(SESSION_HANDOFF, "Hand Off Workspace…", None)?)
        .item(&item(
            SESSION_SHARE_REPLAY,
            "Share Encrypted Replay…",
            None,
        )?)
        .separator()
        .item(&item(SESSION_ARCHIVE, "Archive Session", None)?)
        .build()?;

    // ---- View -------------------------------------------------------------------------
    // Appearance uses plain items rather than check items on purpose: the theme preference
    // lives in the webview's ThemeProvider, so a checkmark here could only ever be a guess that
    // drifts out of sync with the app (and with the OS, under "System").
    let appearance = SubmenuBuilder::new(app, "Appearance")
        .item(&item(VIEW_APPEARANCE_LIGHT, "Light", None)?)
        .item(&item(VIEW_APPEARANCE_DARK, "Dark", None)?)
        .item(&item(VIEW_APPEARANCE_SYSTEM, "System", None)?)
        .build()?;

    let view = SubmenuBuilder::new(app, "View")
        .item(&item(
            VIEW_SIDEBAR,
            "Toggle Sidebar",
            Some("CmdOrCtrl+Backslash"),
        )?)
        .item(&item(
            VIEW_SPLIT_PANE,
            "Split Pane Right",
            Some("CmdOrCtrl+D"),
        )?)
        .item(&item(VIEW_TERMINAL, "Terminal Dock", Some("CmdOrCtrl+J"))?)
        .item(&item(VIEW_USAGE, "Usage Panel", Some("CmdOrCtrl+U"))?)
        .item(&item(VIEW_NOTES, "Notes", None)?)
        .item(&item(VIEW_GIT_REVIEW, "Git Review", Some("CmdOrCtrl+G"))?)
        .item(&item(VIEW_BROWSER_PREVIEW, "Browser Preview", None)?)
        .separator()
        .item(&appearance)
        .build()?;

    // ---- Window / Help ----------------------------------------------------------------
    let window = SubmenuBuilder::new(app, "Window")
        .minimize()
        .maximize()
        .fullscreen()
        .separator()
        .close_window()
        .build()?;

    let help = SubmenuBuilder::new(app, "Help")
        .item(&item(HELP_DOCS, "Forge Documentation", None)?)
        .item(&item(HELP_ISSUE, "Report an Issue…", None)?)
        .separator()
        .item(&item(HELP_ACKNOWLEDGEMENTS, "Acknowledgements", None)?)
        .build()?;

    let menu = Menu::with_items(
        app,
        &[&forge, &file, &edit, &session, &view, &window, &help],
    )?;
    app.set_menu(menu)?;
    Ok(())
}

/// Raises the main window from hidden/minimised/behind-another-app back to the front. Since
/// `lib.rs` hides rather than closes it, this is the only route back to the app once the user
/// has closed the window — the tray's own items, and the macOS Dock reopen request.
pub fn raise_main_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Single dispatch point for both the application menu and the tray menu (Tauri delivers those
/// to two different handlers, but they share one id vocabulary).
pub fn handle_event<R: Runtime>(app: &AppHandle<R>, id: &str) {
    match id {
        // Handled natively: the About window must open even when the webview is unpaired or
        // still booting, which is exactly when a user reaches for "About".
        APP_ABOUT => {
            let _ = about::open(app);
            return;
        }
        // The one deliberate exit. `AppHandle::exit` is the only path `lib.rs`'s
        // `ExitRequested` guard treats as a real quit.
        APP_QUIT => {
            app.exit(0);
            return;
        }
        #[cfg(debug_assertions)]
        DEV_RELOAD => {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.eval("window.location.reload()");
            }
            return;
        }
        _ => {}
    }

    // A tray item can be picked while the window is hidden, minimised, or behind another app;
    // raising it is a native-side job the webview cannot do for itself before it has focus.
    if id.starts_with("tray:") {
        raise_main_window(app);
    }

    let _ = app.emit(MENU_EVENT, MenuEventPayload { id: id.to_string() });
}
