import { describe, expect, it } from "vitest";

import { anywhereConsumersReady } from "./anywhereStartup";

describe("Anywhere managed transport startup", () => {
  it("holds consumers until the current relay runtime is registered", () => {
    const runtime = "account:device:epoch";

    expect(anywhereConsumersReady("ready", runtime, null)).toBe(false);
    expect(anywhereConsumersReady("ready", runtime, runtime)).toBe(true);
  });

  it("allows signed-out direct-mode consumers without a relay runtime", () => {
    expect(anywhereConsumersReady("signed_out", null, null)).toBe(true);
  });
});
