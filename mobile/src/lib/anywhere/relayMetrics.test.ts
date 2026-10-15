import { beforeEach, describe, expect, it } from "vitest";

import { getColdStartTimings, getRelayMetrics, markColdStart, recordInboundHandling, recordRelayRoundTrip, resetRelayMetrics } from "./relayMetrics";

describe("relay metrics", () => {
  beforeEach(resetRelayMetrics);

  it("reports empty stats without samples", () => {
    expect(getRelayMetrics().roundTrip).toEqual({ samples: 0, p50Ms: null, p90Ms: null, maxMs: null });
  });

  it("computes percentiles over the last 100 samples only", () => {
    for (let i = 1; i <= 150; i += 1) recordRelayRoundTrip(i);
    const stats = getRelayMetrics().roundTrip;
    expect(stats.samples).toBe(100);
    expect(stats.maxMs).toBe(150);
    expect(stats.p50Ms).toBe(101);
    expect(stats.p90Ms).toBe(141);
  });

  it("keeps round-trip and inbound windows separate", () => {
    recordInboundHandling(5);
    expect(getRelayMetrics().inboundHandling.samples).toBe(1);
    expect(getRelayMetrics().roundTrip.samples).toBe(0);
  });

  it("keeps the first mark per cold-start stage, relative to credentials loading", () => {
    expect(getColdStartTimings()).toEqual({});
    markColdStart("relayOpen");
    expect(getColdStartTimings()).toEqual({});
    markColdStart("credentialsLoaded");
    markColdStart("relayOpen");
    const first = getColdStartTimings().relayOpen;
    markColdStart("relayOpen");
    expect(getColdStartTimings().relayOpen).toBe(first);
    expect(getColdStartTimings().credentialsLoaded).toBe(0);
  });
});
