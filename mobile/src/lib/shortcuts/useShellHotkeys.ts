// Desktop shell shortcuts (Machined wave 2), resolved through the persisted app-action
// registry. Named wrappers keep call sites in shell/ and CommandPalette intent-shaped.
// The platform `useHotkey` twin makes them no-ops on native mobile.
//
// Each wrapper also claims the matching item in the Tauri menu bar (docs/design/machined § 08
// Native): on macOS a menu key equivalent is swallowed by the menu and never reaches the
// webview, so without this registration the accelerators printed in the menu would stop working
// the moment the menu bar was installed. `useDesktopMenuAction` is inert off Tauri.
import { useDesktopMenuAction } from "../desktopMenu";
import { useAppShortcut } from "./useAppShortcut";

/** ⌘\ — collapse/expand the desktop sidebar to the icon rail. */
export function useSidebarCollapseHotkey(onToggle: () => void): void {
  useAppShortcut("shell.sidebar", onToggle);
  useDesktopMenuAction("view:sidebar", onToggle);
}

/** ⌘U — toggle the right-hand usage dock. */
export function useUsageDockHotkey(onToggle: () => void): void {
  useAppShortcut("shell.usage", onToggle);
  useDesktopMenuAction("view:usage", onToggle);
}

/** ⌥Space — open the in-window quick composer. */
export function useQuickComposerHotkey(onOpen: () => void): void {
  useAppShortcut("shell.quickComposer", onOpen);
  useDesktopMenuAction("session:quick-composer", onOpen);
  // The tray's footer offers the same action while the window is in the background.
  useDesktopMenuAction("tray:quick-composer", onOpen);
}

/** ⌘P — open the command palette straight into thread-search mode. */
export function useThreadSearchHotkey(onOpen: () => void): void {
  useAppShortcut("session.search", onOpen);
  useDesktopMenuAction("session:search", onOpen);
}
