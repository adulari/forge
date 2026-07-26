// When the fleet list is allowed to go back to the network.
//
// Pulled out of `useSessions` so it can be tested without a renderer: the bug it encodes was
// invisible in every unit test and only showed up as a flicker on a phone.

/** Slow safety net for a dropped websocket event, not the primary update path. */
export const SESSIONS_RECOVERY_POLL_MS = 60_000;

export interface SessionsRefetchPolicy {
  /** `"always"` ignores staleTime — every mount goes to the network. */
  refetchOnMount: "always" | false;
  refetchInterval: number | false;
  refetchOnWindowFocus: boolean;
}

/**
 * `peeking` is the copy of a tab that TabPager draws during a swipe: on screen for the length of a
 * drag, then discarded. Treating that as an arrival made every swipe between Fleet and Inbox fire a
 * round trip, and the reload it triggered was visible as a flicker mid-gesture — followed by a
 * second one when the real screen mounted.
 */
export function sessionsRefetchPolicy(
  { peeking, isFocused }: { peeking: boolean; isFocused: boolean },
): SessionsRefetchPolicy {
  if (peeking) {
    return { refetchOnMount: false, refetchInterval: false, refetchOnWindowFocus: false };
  }
  return {
    refetchOnMount: "always",
    refetchInterval: isFocused ? SESSIONS_RECOVERY_POLL_MS : false,
    refetchOnWindowFocus: true,
  };
}
