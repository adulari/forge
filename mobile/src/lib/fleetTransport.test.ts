import { describe, expect, it } from "vitest";

import { supportsFleetInvalidationSocket } from "./fleetTransport";

describe("fleet invalidation transport capability", () => {
  it("uses the socket for direct daemons", () => {
    expect(supportsFleetInvalidationSocket("https://forge.local/token")).toBe(true);
  });

  it("falls back to session polling for managed Anywhere", () => {
    expect(supportsFleetInvalidationSocket(`fany://${"a".repeat(32)}`)).toBe(false);
  });
});
