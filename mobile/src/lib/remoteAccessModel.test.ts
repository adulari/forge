import { describe, expect, it } from "vitest";

import type { AccessAddress, PairedClient } from "./apiAccess";
import { accessFrom, defaultAddressKind, FULL_ACCESS, lastConnectedLabel, pairingLink, rotateAfterSeconds, scopeSummary } from "./remoteAccessModel";

const client = (over: Partial<PairedClient>): PairedClient => ({
  id: "a1",
  label: "Pixel",
  scopes: ["view", "prompt", "approve", "terminal", "workspace-write", "manage-providers"],
  created_at: 0,
  last_seen: null,
  pending: false,
  preset: "full",
  ...over,
});

describe("remote access model", () => {
  it("labels last connected from the host clock", () => {
    const now = 10_000_000;
    expect(lastConnectedLabel(client({ pending: true }), now)).toBe("Waiting to pair");
    expect(lastConnectedLabel(client({}), now)).toBe("Never connected");
    expect(lastConnectedLabel(client({ last_seen: 9_999 }), now)).toBe("Just now");
    expect(lastConnectedLabel(client({ last_seen: 9_000 }), now)).toBe("16m ago");
    expect(lastConnectedLabel(client({ last_seen: 9_000 - 7200 }), now)).toBe("2h ago");
    expect(lastConnectedLabel(client({ last_seen: 9_000 - 3 * 86_400 }), now)).toBe("3d ago");
  });

  it("summarises scopes", () => {
    expect(scopeSummary(client({}))).toBe("Full access");
    expect(scopeSummary(client({ scopes: ["view"], preset: "viewer" }))).toBe("View only");
    expect(scopeSummary(client({ scopes: ["view", "prompt"] }))).toBe("View, Prompt");
  });

  it("builds a pairing link only from a reachable address", () => {
    const lan: AccessAddress = { kind: "lan", base: "https://192.168.1.5:7420/", note: "" };
    expect(pairingLink(lan, "tok")).toBe("https://192.168.1.5:7420/tok");
    expect(pairingLink({ kind: "public", base: null, note: "" }, "tok")).toBeNull();
    expect(pairingLink(lan, null)).toBeNull();
  });

  it("derives access from whoami and defaults to full for older hosts", () => {
    expect(accessFrom(null)).toBe(FULL_ACCESS);
    const viewer = accessFrom({ id: "x", label: "tv", owner: false, scopes: ["view"] });
    expect(viewer).toMatchObject({ viewOnly: true, canPrompt: false, canApprove: false, owner: false });
    expect(accessFrom({ id: null, label: "owner", owner: true, scopes: ["view", "prompt", "approve"] }).viewOnly).toBe(false);
  });

  it("picks the first reachable address for the mode", () => {
    const addresses: AccessAddress[] = [
      { kind: "local", base: "http://127.0.0.1:7420", note: "" },
      { kind: "lan", base: null, note: "" },
      { kind: "tailscale", base: null, note: "" },
      { kind: "public", base: "https://x.trycloudflare.com", note: "" },
    ];
    expect(defaultAddressKind(addresses, "local")).toBe("local");
    expect(defaultAddressKind(addresses, "anywhere")).toBe("public");
    expect(defaultAddressKind(addresses, "lan")).toBe("local");
  });

  it("rotates a code before the host drops it", () => {
    expect(rotateAfterSeconds(600)).toBe(570);
    expect(rotateAfterSeconds(40)).toBe(15);
  });
});
