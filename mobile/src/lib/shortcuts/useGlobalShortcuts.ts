// Global desktop/web navigation actions. Defaults are Alt+1..4 for the four tab routes
// and Cmd/Ctrl+N for New Session, but `useAppShortcut` resolves persisted overrides.
// Native's `useHotkeys.ts` twin keeps the calls inert on touch-only mobile surfaces.
import { router } from "expo-router";

import { useDesktopMenu } from "../desktopMenu";
import { useAppShortcut } from "./useAppShortcut";

const TAB_ROUTES = ["/", "/inbox", "/history", "/settings"] as const;

export function useGlobalShortcuts(): void {
  // The Tauri menu bar / tray event bridge (docs/design/machined § 08 Native). Mounted here
  // because this hook is already the app-root, once-per-launch shortcut host, and the bridge
  // needs no context; it is inert off Tauri.
  useDesktopMenu();

  useAppShortcut("nav.fleet", () => router.push(TAB_ROUTES[0]));
  useAppShortcut("nav.inbox", () => router.push(TAB_ROUTES[1]));
  useAppShortcut("nav.history", () => router.push(TAB_ROUTES[2]));
  useAppShortcut("nav.settings", () => router.push(TAB_ROUTES[3]));
  useAppShortcut("session.new", () => router.push("/new-session"));
}
