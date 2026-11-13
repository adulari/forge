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

/** Anywhere (relay) hosts have no fleet socket, so the poll is their only update path. */
export const SESSIONS_RELAY_POLL_MS = 15_000;

/** How often a host that is not the active one is polled just to notice waiting sessions. */
export const OTHER_HOST_WATCH_POLL_MS = 15_000;
/** Other hosts' poll while none of their sessions is busy or waiting: an idle session cannot start
 * waiting without a new prompt, so the fast poll would only spend battery and relay traffic. */
export const OTHER_HOST_IDLE_POLL_MS = 60_000;

export interface RefetchPolicy {
  /** `"always"` ignores staleTime — every mount goes to the network. */
  refetchOnMount: "always" | boolean;
  refetchInterval: number | false;
  refetchOnWindowFocus: boolean;
}

/**
 * A relay hiccup right after connect is usually gone within a moment, so the first retry is fast
 * rather than react-query's default 1 s / 2 s / 4 s ladder.
 */
export const sessionsRetryPolicy = {
  retry: 2,
  retryDelay: (attempt: number): number => (attempt === 0 ? 300 : 1000),
};

/** The live fleet list, which both Fleet and Inbox render. */
export function sessionsRefetchPolicy(
  { peeking, isFocused, hasFleetSocket = true }: { peeking: boolean; isFocused: boolean; hasFleetSocket?: boolean },
): RefetchPolicy {
  if (peeking) {
    return { refetchOnMount: false, refetchInterval: false, refetchOnWindowFocus: false };
  }
  return {
    refetchOnMount: "always",
    refetchInterval: isFocused ? (hasFleetSocket ? SESSIONS_RECOVERY_POLL_MS : SESSIONS_RELAY_POLL_MS) : false,
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
