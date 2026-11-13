import { describe, expect, it } from "vitest";

import { outageClock, providerOutageBanner } from "./providerOutage";
import type { SnapshotOutage } from "./ws";

const outage: SnapshotOutage = {
  model: "inferhub::deepseek-v4.1-flash",
  provider: "inferhub",
  since_unix: 1_762_420_680,
  next_try_unix: 1_762_422_180,
  until_unix: 1_762_449_480,
  attempts: 4,
  reason: "503 no_capacity",
};

describe("providerOutageBanner", () => {
  it("is absent while the provider answers", () => {
    expect(providerOutageBanner(undefined)).toBeNull();
    expect(providerOutageBanner(null)).toBeNull();
  });

  it("says when the provider went down, the next try and the end of the wait", () => {
    const banner = providerOutageBanner(outage);
    expect(banner?.tone).toBe("warn");
    expect(banner?.message).toBe(
      `inferhub down since ${outageClock(outage.since_unix)} (503 no_capacity). ` +
        `Next try ${outageClock(outage.next_try_unix)}, giving up ${outageClock(outage.until_unix)}. ` +
        "Stop the turn to end the wait.",
    );
    expect(banner?.message).toMatch(/down since \d{2}:\d{2}/);
  });

  it("falls back to the model when the host sent no provider name", () => {
    expect(providerOutageBanner({ ...outage, provider: "" })?.message).toMatch(/^inferhub::deepseek/);
  });
});
