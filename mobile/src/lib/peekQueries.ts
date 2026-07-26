// When a list is allowed to go back to the network.
//
// Pulled out of the hooks so the decision is testable without a renderer: the bug it encodes was
// invisible in every unit test and only showed up as a reload flicker on a phone.
//
// `peeking` is the copy of a tab the pager draws during a swipe — on screen for the length of a
// drag, then discarded. Treating that as an arrival made every swipe fire a round trip, and the
// reload it triggered was visible mid-gesture, followed by another when the real screen mounted.
//
// Note `useIsFocused()` cannot be used to tell the two apart: a peek is rendered INSIDE the focused
// route, so focus is true for both.

/** Slow safety net for a dropped websocket event, not the primary update path. */
export const SESSIONS_RECOVERY_POLL_MS = 60_000;

export interface RefetchPolicy {
  /** `"always"` ignores staleTime — every mount goes to the network. */
  refetchOnMount: "always" | boolean;
  refetchInterval: number | false;
  refetchOnWindowFocus: boolean;
}

/** The live fleet list, which both Fleet and Inbox render. */
export function sessionsRefetchPolicy(
  { peeking, isFocused }: { peeking: boolean; isFocused: boolean },
): RefetchPolicy {
  if (peeking) {
    return { refetchOnMount: false, refetchInterval: false, refetchOnWindowFocus: false };
  }
  return {
    refetchOnMount: "always",
    refetchInterval: isFocused ? SESSIONS_RECOVERY_POLL_MS : false,
    refetchOnWindowFocus: true,
  };
}

/**
 * History's past-sessions list. It has no poll of its own and never forced a refetch, so the only
 * thing to suppress is the window-focus one — but a peeked History was still the most expensive
 * screen to slide past, because an infinite query with no cached page fetches on mount regardless
 * of this policy. Keeping it honest here means a second swipe costs nothing.
 */
export function pastSessionsRefetchPolicy({ peeking }: { peeking: boolean }): RefetchPolicy {
  return peeking
    ? { refetchOnMount: false, refetchInterval: false, refetchOnWindowFocus: false }
    : { refetchOnMount: true, refetchInterval: false, refetchOnWindowFocus: true };
}
