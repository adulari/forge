import { describe, expect, it, vi } from "vitest";

import {
  anywherePushStatus,
  disableAnywherePush,
  enableAnywherePush,
  observeAnywherePush,
  type AnywherePushApi,
  type AnywherePushPlatform,
  type AnywherePushRegistration,
  type AnywherePushStorage,
} from "./anywherePushCore";

function harness(permission: "granted" | "denied" | "undetermined" = "granted") {
  let stored: AnywherePushRegistration | null = null;
  let refresh: (() => void) | null = null;
  const platform: AnywherePushPlatform = {
    supported: () => true,
    permission: vi.fn(async () => permission),
    requestPermission: vi.fn(async () => "granted" as const),
    deviceToken: vi.fn(async () => "a".repeat(64)),
    environment: () => "production",
    observeRefresh: vi.fn((callback) => {
      refresh = callback;
      return () => { refresh = null; };
    }),
  };
  const storage: AnywherePushStorage = {
    load: vi.fn(async () => stored),
    save: vi.fn(async (next) => { stored = next; }),
    clear: vi.fn(async () => { stored = null; }),
  };
  const api: AnywherePushApi = {
    register: vi.fn(async () => ({ subscription_id: "b".repeat(32) })),
    revoke: vi.fn(async () => undefined),
  };
  return { platform, storage, api, stored: () => stored, refresh: () => refresh };
}

describe("Anywhere generic push", () => {
  it("registers an ephemeral APNs token and stores only an opaque subscription id", async () => {
    const test = harness();
    await expect(enableAnywherePush(test.platform, test.storage, test.api)).resolves.toBe("subscribed");
    expect(test.api.register).toHaveBeenCalledWith({
      platform: "apns",
      environment: "production",
      device_token: "a".repeat(64),
    });
    expect(test.stored()).toEqual({ subscriptionId: "b".repeat(32), environment: "production" });
    expect(JSON.stringify(test.stored())).not.toContain("a".repeat(64));
    await expect(anywherePushStatus(test.platform, test.storage)).resolves.toBe("subscribed");
  });

  it("does not register when notification permission is denied", async () => {
    const test = harness("denied");
    await expect(enableAnywherePush(test.platform, test.storage, test.api)).resolves.toBe("denied");
    expect(test.api.register).not.toHaveBeenCalled();
  });

  it("retains local revocation state until the service accepts the revoke", async () => {
    const test = harness();
    await enableAnywherePush(test.platform, test.storage, test.api);
    vi.mocked(test.api.revoke).mockRejectedValueOnce(new Error("offline"));
    await expect(disableAnywherePush(test.platform, test.storage, test.api)).rejects.toThrow("offline");
    expect(test.stored()).not.toBeNull();
    await expect(disableAnywherePush(test.platform, test.storage, test.api)).resolves.toBe("unsubscribed");
    expect(test.stored()).toBeNull();
  });

  it("treats receipt and open as content-free refresh hints", () => {
    const test = harness();
    const onRefresh = vi.fn();
    const remove = observeAnywherePush(test.platform, onRefresh);
    test.refresh()?.();
    expect(onRefresh).toHaveBeenCalledWith();
    remove();
    expect(test.refresh()).toBeNull();
  });
});
