// Back/close affordances can be reached with nothing on the stack to pop (deep link,
// PWA cold start, or a screen entered via `router.replace`) — calling `router.back()` then
// warns "GO_BACK was not handled by any navigator". Guard with `canGoBack()` and fall back
// to a sensible parent route instead.
import { router } from "expo-router";
import { Platform } from "react-native";

import { activateSession, getActivationOverride } from "./sessionPrewarm";

export function goBackOr(fallback: Parameters<typeof router.replace>[0]) {
  if (router.canGoBack()) {
    router.back();
  } else {
    router.replace(fallback);
  }
}

// Settings hides the sidebar on a wide window, so its rail needs its own way out. The shell
// records the last route outside Settings and the rail's back link returns there.
let lastShellRoute: string | null = null;

export function rememberShellRoute(pathname: string): void {
  lastShellRoute = pathname;
}

// Wide-window breakpoint (theme/useBreakpoint "expanded"): the desktop shell, whose session screen
// has no Back button, so a stack of earlier sessions is never reachable.
const EXPANDED_MIN_WIDTH = 1024;

/**
 * Open a session from the rail, the palette or the quick composer. On the desktop shell a session
 * opened from another session REPLACES it instead of pushing a new screen: every pushed session
 * stayed mounted underneath (socket, transcript, composer, ~70 ms of re-render per snapshot frame
 * each), so the app got slower with every session you had visited. Phones and narrow windows keep
 * the push, where Back returns to the previous session.
 */
export function openSession(id: string): void {
  const href = `/session/${id}` as Parameters<typeof router.push>[0];
  const current = lastShellRoute ?? "";
  const inSession = current.startsWith("/session/");
  const wide = Platform.OS === "web" && typeof window !== "undefined" && window.innerWidth >= EXPANDED_MIN_WIDTH;
  if (!inSession || !wide) {
    router.push(href);
    return;
  }
  const openId = getActivationOverride() ?? current.match(/^\/session\/([^/]+)/)?.[1];
  if (openId === id) return;
  activateSession(id);
}

export function leaveSettings(): void {
  router.replace((lastShellRoute ?? "/") as Parameters<typeof router.replace>[0]);
}
