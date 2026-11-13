import type { SnapshotOutage } from "./ws";

export interface ProviderOutageBanner {
  tone: "warn";
  message: string;
}

/** Local `HH:MM` for a unix-seconds timestamp. */
export function outageClock(unixSecs: number): string {
  return new Date(unixSecs * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", hour12: false });
}

/**
 * The banner for a turn that is waiting out a provider outage, from `snapshot.outage`. The turn is
 * not stuck: the host re-sends the request on a slow schedule and carries on when the provider
 * answers, so the banner says when it went down, when it tries next and when it gives up.
 */
export function providerOutageBanner(outage: SnapshotOutage | null | undefined): ProviderOutageBanner | null {
  if (!outage) return null;
  const provider = outage.provider || outage.model;
  return {
    tone: "warn",
    message:
      `${provider} down since ${outageClock(outage.since_unix)} (${outage.reason}). ` +
      `Next try ${outageClock(outage.next_try_unix)}, giving up ${outageClock(outage.until_unix)}. ` +
      "Stop the turn to end the wait.",
  };
}
