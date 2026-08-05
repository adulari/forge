import { isTauri } from "./platform";

interface BrowserAuthWindow {
  closed: boolean;
  close(): void;
  location: { replace(url: string): void };
  opener: unknown;
}

type OpenBrowserWindow = (
  url?: string,
  target?: string,
  features?: string,
) => BrowserAuthWindow | null;

export interface ReservedBrowserAuthWindow {
  navigate(url: string): void;
  close(): void;
}

export async function runReservedBrowserFlow<T>(
  reserved: ReservedBrowserAuthWindow | null,
  work: () => Promise<T>,
): Promise<T> {
  try {
    return await work();
  } catch (reason) {
    reserved?.close();
    throw reason;
  }
}

function defaultOpenWindow(url?: string, target?: string, features?: string): BrowserAuthWindow | null {
  if (typeof window === "undefined") return null;
  return window.open(url, target, features);
}

/**
 * Reserve a tab synchronously while a button press still carries browser user activation.
 * The caller can navigate it after the device-flow request returns without unloading Forge.
 */
export function reserveBrowserAuthWindow(
  openWindow: OpenBrowserWindow = defaultOpenWindow,
): ReservedBrowserAuthWindow | null {
  const popup = openWindow("about:blank", "_blank");
  if (!popup) return null;
  popup.opener = null;
  return {
    navigate(url) {
      if (!popup.closed) popup.location.replace(url);
    },
    close() {
      if (!popup.closed) popup.close();
    },
  };
}

/** Open a retry link in a separate tab; never replace the polling Forge page. */
export function openBrowserAuthUrl(
  url: string,
  openWindow: OpenBrowserWindow = defaultOpenWindow,
): void {
  openWindow(url, "_blank", "noopener,noreferrer");
}

/**
 * Open an external URL from whichever shell is running.
 *
 * `Platform.OS` is `"web"` under Tauri too, because the desktop shell renders the same
 * react-native-web bundle. But WebKitGTK has no popup support, so `window.open` returns null
 * there: both the reserved-tab path and the retry link silently no-op and the user is left
 * staring at a code with no browser. The opener plugin is the only route that works, and
 * `opener:default` is already granted in src-tauri/capabilities/default.json.
 */
export async function openExternalAuthUrl(
  url: string,
  openWindow: OpenBrowserWindow = defaultOpenWindow,
): Promise<void> {
  if (isTauri) {
    const { openUrl } = await import("@tauri-apps/plugin-opener");
    await openUrl(url);
    return;
  }
  openBrowserAuthUrl(url, openWindow);
}
