import { describe, expect, it } from "vitest";

import {
  anywhereConsumersReady,
  phaseAfterSetupRestart,
} from "./anywhereStartup";

describe("Anywhere managed transport startup", () => {
  it("holds consumers until the current relay runtime is registered", () => {
    const runtime = "account:device:epoch";

    expect(anywhereConsumersReady("ready", runtime, null)).toBe(false);
    expect(anywhereConsumersReady("ready", runtime, runtime)).toBe(true);
  });

  it("allows signed-out direct-mode consumers without a relay runtime", () => {
    expect(anywhereConsumersReady("signed_out", null, null)).toBe(true);
  });

  it("allows direct-mode consumers while a stale managed session reconnects", () => {
    expect(anywhereConsumersReady("reauthentication_required", "stale-runtime", null)).toBe(true);
  });

  it("does not turn retained invalid credentials back into a ready session", () => {
    expect(phaseAfterSetupRestart(true, true)).toBe("reauthentication_required");
    expect(phaseAfterSetupRestart(true, false)).toBe("ready");
    expect(phaseAfterSetupRestart(false, false)).toBe("signed_out");
  });
});
