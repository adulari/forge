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

export function notifySupported(): boolean {
  if (isTauri) return true;
  return isWeb && typeof window !== "undefined" && "Notification" in window;
}

async function notifyTauri(title: string, body?: string): Promise<void> {
  const { isPermissionGranted, requestPermission, sendNotification } = await import(
    "@tauri-apps/plugin-notification"
  );
  let granted = await isPermissionGranted();
  if (!granted) {
    granted = (await requestPermission()) === "granted";
  }
  if (granted) sendNotification({ title, body });
}

async function notifyWeb(title: string, body?: string): Promise<void> {
  if (typeof window === "undefined" || !("Notification" in window)) return;
  let permission = Notification.permission;
  if (permission === "default") {
    permission = await Notification.requestPermission();
  }
  if (permission === "granted") {
    new Notification(title, { body });
  }
}

/** Best-effort system notification. Never throws — callers don't need a try/catch. */
export async function notify(title: string, body?: string): Promise<void> {
  try {
    if (isTauri) {
      await notifyTauri(title, body);
    } else if (isWeb) {
      await notifyWeb(title, body);
    }
  } catch {
    // best-effort — the in-app toast already showed the message
  }
}
