// Runtime platform flags. ARCHITECTURE.md §6.2 — `isTauri` is checked once here; the three
// Tauri-only branches (transport §6.3, notifications, external-link opening) key off it.
import { Platform } from "react-native";

export const isTauri =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export const isWeb = Platform.OS === "web";
export const isNative = !isWeb;
export const isIOS = Platform.OS === "ios";

// macOS host detection for the Tauri desktop shell (titlebar drag region — only macOS's
// "Overlay" titleBarStyle needs a manual data-tauri-drag-region strip; Windows/Linux keep
// their normal decorated title bar). `navigator.platform` is deprecated but still widely
// supported and simplest here; `userAgent` is the fallback for engines that already blanked
// `platform`.
export const isMacOS =
  typeof navigator !== "undefined" && /Mac/.test(navigator.platform || navigator.userAgent || "");
