// System notification feature-detect (ARCHITECTURE.md §6.1: "native notifications
// (tauri-plugin-notification, triggered from the same code path that calls web
// Notification — feature-detected)"). This is the one call site both branches share;
// `ToastHost` fires it when the toast is shown while the window/tab isn't focused.
//
// - Tauri: `@tauri-apps/plugin-notification`, lazy-imported so web/native bundles never
//   load it.
// - Web (non-Tauri): the browser `Notification` API directly.
// - Native: no-op — v1 has no local-notification story on iOS/Android (native APNs is a
//   flagged backend gap, ARCHITECTURE.md §7).
import { isTauri, isWeb } from "./platform";

export type NotifyPermission = "granted" | "denied" | "default" | "unsupported";

/** Last-observed permission/failure state, for Settings to read (§ FIX 5b — `notify()`
 * is best-effort and never throws, so this is the only way a caller finds out a
 * notification silently didn't go out because permission was denied). */
let lastPermission: NotifyPermission = "default";

export function getNotifyPermission(): NotifyPermission {
  return lastPermission;
}

export function notifySupported(): boolean {
  if (isTauri) return true;
  return isWeb && typeof window !== "undefined" && "Notification" in window;
}

async function notifyTauri(title: string, body?: string): Promise<void> {
  const { isPermissionGranted, requestPermission, sendNotification } = await import(
    "@tauri-apps/plugin-notification"
  );
  let granted = await isPermissionGranted();
  if (granted) {
    lastPermission = "granted";
  } else {
    // requestPermission() (unlike isPermissionGranted()) returns the real three-state
    // NotificationPermission ("granted" | "denied" | "default"), so this is the only
    // point where we can honestly learn "denied" rather than just "not granted".
    const permission = await requestPermission();
    lastPermission = permission;
    granted = permission === "granted";
  }
  if (granted) sendNotification({ title, body });
}

async function notifyWeb(title: string, body?: string): Promise<void> {
  if (typeof window === "undefined" || !("Notification" in window)) {
    lastPermission = "unsupported";
    return;
  }
  let permission = Notification.permission;
  if (permission === "default") {
    permission = await Notification.requestPermission();
  }
  lastPermission = permission;
  if (permission === "granted") {
    new Notification(title, { body });
  }
}

/** Best-effort system notification. Never throws — callers don't need a try/catch.
 * Check `getNotifyPermission()` after a call if you need to know whether it actually
 * went out (e.g. to show a "notifications are blocked" hint in Settings). */
export async function notify(title: string, body?: string): Promise<void> {
  try {
    if (isTauri) {
      await notifyTauri(title, body);
    } else if (isWeb) {
      await notifyWeb(title, body);
    } else {
      lastPermission = "unsupported";
    }
  } catch {
    // best-effort — the in-app toast already showed the message. We don't know which
    // side (permission check vs. send) failed, so leave `lastPermission` at whatever it
    // was before this call rather than guessing "denied".
  }
}

/**
 * Tauri-only: refresh the current OS notification permission without prompting or
 * sending anything. Used by Settings to show the current state on mount, before the
 * user has triggered a test notification.
 *
 * NOTE: `isPermissionGranted()` only returns a boolean — it can't distinguish "denied"
 * from "never asked" the way `requestPermission()`'s three-state result can (and calling
 * requestPermission() here would prompt the user just for viewing Settings, which is
 * the wrong UX). So a `false` result only ever downgrades a stale "granted" to the
 * honest "default" (unknown); it never claims "denied" — that label is only set from
 * `notify()`'s actual `requestPermission()` call, e.g. via the "send test notification"
 * action.
 */
export async function checkNotifyPermission(): Promise<NotifyPermission> {
  if (!isTauri) return lastPermission;
  try {
    const { isPermissionGranted } = await import("@tauri-apps/plugin-notification");
    const granted = await isPermissionGranted();
    if (granted) {
      lastPermission = "granted";
    } else if (lastPermission === "granted") {
      lastPermission = "default";
    }
  } catch {
    // best-effort — leave lastPermission as-is
  }
  return lastPermission;
}
