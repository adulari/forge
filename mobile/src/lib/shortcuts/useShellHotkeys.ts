// Desktop shell shortcuts (Machined wave 2), layered on the same `useHotkey` registry
// T5.1 wired ⌘K/⌘1..4/⌘N through — named wrappers so call sites in shell/ and
// CommandPalette read as intent, matching `usePaletteHotkey`'s convention. Each is a
// thin `useHotkey` call, so it's a no-op on native for free (useHotkey's own platform
// twin handles that) — no branching needed here.
//
// Each wrapper also claims the matching item in the Tauri menu bar (docs/design/machined § 08
// Native): on macOS a menu key equivalent is swallowed by the menu and never reaches the
// webview, so without this registration the accelerators printed in the menu would stop working
// the moment the menu bar was installed. `useDesktopMenuAction` is inert off Tauri.
import { useDesktopMenuAction } from "../desktopMenu";
import { useHotkey } from "./useHotkeys";

/** ⌘\ — collapse/expand the desktop sidebar to the icon rail. */
export function useSidebarCollapseHotkey(onToggle: () => void): void {
  useHotkey("\\", onToggle, { meta: true });
  useDesktopMenuAction("view:sidebar", onToggle);
}

/** ⌘U — toggle the right-hand usage dock. */
export function useUsageDockHotkey(onToggle: () => void): void {
  useHotkey("u", onToggle, { meta: true });
  useDesktopMenuAction("view:usage", onToggle);
}

/** ⌥Space — open the in-window quick composer. */
export function useQuickComposerHotkey(onOpen: () => void): void {
  useHotkey(" ", onOpen, { alt: true });
  useDesktopMenuAction("session:quick-composer", onOpen);
  // The tray's footer offers the same action while the window is in the background.
  useDesktopMenuAction("tray:quick-composer", onOpen);
}

/** ⌘P — open the command palette straight into thread-search mode. */
export function useThreadSearchHotkey(onOpen: () => void): void {
  useHotkey("p", onOpen, { meta: true });
  useDesktopMenuAction("session:search", onOpen);
}
