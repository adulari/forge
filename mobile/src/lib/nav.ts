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
