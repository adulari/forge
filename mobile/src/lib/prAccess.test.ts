import { describe, expect, it, vi } from "vitest";

vi.mock("./api", () => ({ getIdentity: vi.fn() }));
vi.mock("./auth", () => ({ useAuth: () => ({ baseUrl: null }) }));
vi.mock("./transport", () => ({
  supportsDirectDaemonEndpoints: (url: string) => !/^fany(-ws)?:/.test(url),
}));
vi.mock("@tanstack/react-query", () => ({ useQuery: vi.fn() }));

import { SHIP_OFF_MESSAGE, UPDATE_HOST_MESSAGE, prAccessFor, prReadable, shipGate } from "./prAccess";

const DIRECT = "https://forge.test/tok";
const RELAY = "fany://host-1";

describe("pr access", () => {
  it("is fully available on a direct connection without asking the host", () => {
    const access = prAccessFor(DIRECT, undefined, false);
    expect(prReadable(access)).toBe(true);
    expect(shipGate(access)).toEqual({ allowed: true, message: null });
  });

  it("is unavailable with no connection", () => {
    const access = prAccessFor(null, undefined, false);
    expect(prReadable(access)).toBe(false);
    expect(shipGate(access).allowed).toBe(false);
  });

  it("reads PRs on a bridged host that reports the capability, ship off by default", () => {
    const access = prAccessFor(RELAY, { ship: { pr: true, over_relay: false } }, false);
    expect(prReadable(access)).toBe(true);
    expect(shipGate(access)).toEqual({ allowed: false, message: SHIP_OFF_MESSAGE });
    expect(SHIP_OFF_MESSAGE).toContain("Ship over Anywhere is off on this host");
    expect(SHIP_OFF_MESSAGE).toContain("allow_ship_over_relay");
  });

  it("allows ship once the host opted in", () => {
    const access = prAccessFor(RELAY, { ship: { pr: true, over_relay: true } }, false);
    expect(prReadable(access)).toBe(true);
    expect(shipGate(access)).toEqual({ allowed: true, message: null });
  });

  it("tells an older host (no ship capability) to be updated", () => {
    const access = prAccessFor(RELAY, {}, false);
    expect(prReadable(access)).toBe(false);
    expect(shipGate(access)).toEqual({ allowed: false, message: UPDATE_HOST_MESSAGE });
  });

  it("waits while the capability is still being read", () => {
    const access = prAccessFor(RELAY, undefined, true);
    expect(prReadable(access)).toBe(false);
    expect(shipGate(access)).toEqual({ allowed: false, message: null });
  });
});
