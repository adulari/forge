// System notification feature-detect (ARCHITECTURE.md §6.1: "native notifications
// (tauri-plugin-notification, triggered from the same code path that calls web
// Notification — feature-detected)"). This is the one call site both branches share;
// `ToastHost` fires it when the toast is shown while the window/tab isn't focused.
//
// - Tauri: `@tauri-apps/plugin-notification`, lazy-imported so web/native bundles never
//   load it. The OS offers no click event on desktop, so a click there only brings the window up.
// - Web (non-Tauri): the browser `Notification` API directly; a click focuses the tab and runs
//   the caller's `onClick`.
// - Native: no-op here — iOS and Android raise theirs through `lib/notices/localNotify`.
// Text is stripped of control sequences and clipped (80 / 240) on the way out.
import { getDesktopShellPrefs } from "./desktopShellPrefs";
import { getNotifyPrefs } from "./notifyPrefs";
import { clipNotification } from "./notifyPrefsCore";
import { playAlertChime } from "./notifySound";
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

async function notifyWeb(title: string, body?: string, onClick?: () => void): Promise<void> {
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
    // Sent silent: the chime (when the Sound setting is on) is played by the app, not the browser.
    const shown = new Notification(title, { body, silent: true });
    if (onClick) {
      shown.onclick = () => {
        window.focus();
        shown.close();
        onClick();
      };
    }
  }
}

/** Best-effort system notification. Never throws — callers don't need a try/catch.
 * Check `getNotifyPermission()` after a call if you need to know whether it actually
 * went out (e.g. to show a "notifications are blocked" hint in Settings). */
export async function notify(
  title: string,
  body?: string,
  options?: { force?: boolean; onClick?: () => void },
): Promise<void> {
  try {
    const clipped = clipNotification(title, body);
    title = clipped.title;
    body = clipped.body;
    if (isTauri) {
      // On by default; the Desktop settings page can switch background notifications off. A
      // test notification from that page (`force`) must still go out so the permission can be seen.
      if (!options?.force && !(await getDesktopShellPrefs()).notifyWhenBackground) return;
      await notifyTauri(title, body);
      if ((await getNotifyPrefs()).sound) playAlertChime();
    } else if (isWeb) {
      await notifyWeb(title, body, options?.onClick);
      if ((await getNotifyPrefs()).sound) playAlertChime();
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
