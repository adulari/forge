import { describe, expect, it } from "vitest";

import type { StoredServer } from "../serverTargets";
import {
  deviceIdentityFingerprint,
  directServerForHost,
  formatReachability,
  hostIdentityFingerprint,
  hostReachability,
} from "./hostIdentity";

const KEY_HEX = "00".repeat(32);

function server(overrides: Partial<StoredServer>): StoredServer {
  return {
    id: "srv_1",
    name: "atlas",
    baseUrl: "http://atlas.local:7777",
    token: "t",
    host: "atlas.local",
    addedAt: 0,
    ...overrides,
  };
}

describe("hostIdentityFingerprint", () => {
  it("is a stable SHA256 digest of the signing key", () => {
    const first = hostIdentityFingerprint(KEY_HEX);
    expect(first).toMatch(/^SHA256:[0-9a-f]{4}…[0-9a-f]{4}$/);
    expect(hostIdentityFingerprint(KEY_HEX)).toBe(first);
    expect(hostIdentityFingerprint("ff".repeat(32))).not.toBe(first);
  });

  it("returns null instead of inventing a value for unknown or malformed keys", () => {
    expect(hostIdentityFingerprint(undefined)).toBeNull();
    expect(hostIdentityFingerprint("")).toBeNull();
    expect(hostIdentityFingerprint("not-hex")).toBeNull();
  });

  it("accepts the base64url form the device list returns", () => {
    expect(deviceIdentityFingerprint("AAAA")).toMatch(/^SHA256:/);
    expect(deviceIdentityFingerprint("!!!")).toBeNull();
    expect(deviceIdentityFingerprint(null)).toBeNull();
  });
});

describe("hostReachability", () => {
  it("always reports the registered relay", () => {
    expect(hostReachability([], "atlas")).toEqual(["anywhere-relay"]);
    expect(formatReachability(hostReachability([], "atlas"))).toBe("anywhere · relay");
  });

  it("adds a direct leg for a saved LAN target with the same daemon hostname", () => {
    const servers = [server({ name: "Atlas" })];
    expect(hostReachability(servers, "atlas")).toEqual(["direct-lan", "anywhere-relay"]);
    expect(formatReachability(hostReachability(servers, "atlas"))).toBe("direct · lan and anywhere · relay");
  });

  it("never matches a managed row against itself", () => {
    const servers = [server({ id: "anywhere:h1", transport: "anywhere", host: "atlas", name: "atlas" })];
    expect(directServerForHost(servers, "atlas")).toBeNull();
    expect(hostReachability(servers, "atlas")).toEqual(["anywhere-relay"]);
  });

  it("matches on the connect URL host and ignores a .local suffix", () => {
    expect(directServerForHost([server({ name: "unnamed" })], "atlas")).not.toBeNull();
    expect(directServerForHost([server({ name: "other", host: "10.0.0.4" })], "atlas")).toBeNull();
  });
});
