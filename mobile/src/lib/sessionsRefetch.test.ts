import { describe, expect, it } from "vitest";

import { sessionsRefetchPolicy, SESSIONS_RECOVERY_POLL_MS } from "./sessionsRefetch";

describe("sessionsRefetchPolicy", () => {
  it("asks for nothing while a tab is only being peeked at", () => {
    // The whole point: a copy of a tab that exists for the length of a swipe must not go to the
    // network. It forced a refetch on mount, mid-gesture, and the reload showed as a flicker.
    expect(sessionsRefetchPolicy({ peeking: true, isFocused: true })).toEqual({
      refetchOnMount: false,
      refetchInterval: false,
      refetchOnWindowFocus: false,
    });
  });

  it("still refetches on a real arrival, which is what keeps a stale fleet honest", () => {
    expect(sessionsRefetchPolicy({ peeking: false, isFocused: true })).toEqual({
      refetchOnMount: "always",
      refetchInterval: SESSIONS_RECOVERY_POLL_MS,
      refetchOnWindowFocus: true,
    });
  });

  it("stops the recovery poll when the screen is not focused", () => {
    const policy = sessionsRefetchPolicy({ peeking: false, isFocused: false });
    expect(policy.refetchInterval).toBe(false);
    // Unfocused is not the same as peeked: arriving still has to catch up.
    expect(policy.refetchOnMount).toBe("always");
  });

  it("lets peeking win over focus, since a peek is rendered inside the focused route", () => {
    // useIsFocused() reports the CURRENT route, and a peek is drawn inside it — so focus is true
    // for a peeked copy and cannot be used to tell the two apart.
    expect(sessionsRefetchPolicy({ peeking: true, isFocused: true }).refetchInterval).toBe(false);
  });
});
