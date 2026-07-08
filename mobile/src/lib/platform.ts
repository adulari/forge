// Runtime platform flags. ARCHITECTURE.md §6.2 — `isTauri` is checked once here; the three
// Tauri-only branches (transport §6.3, notifications, external-link opening) key off it.
import { Platform } from "react-native";

export const isTauri =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export const isWeb = Platform.OS === "web";
export const isNative = !isWeb;
export const isIOS = Platform.OS === "ios";
