// Back/close affordances can be reached with nothing on the stack to pop (deep link,
// PWA cold start, or a screen entered via `router.replace`) — calling `router.back()` then
// warns "GO_BACK was not handled by any navigator". Guard with `canGoBack()` and fall back
// to a sensible parent route instead.
import { router } from "expo-router";

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

export function leaveSettings(): void {
  router.replace((lastShellRoute ?? "/") as Parameters<typeof router.replace>[0]);
}
