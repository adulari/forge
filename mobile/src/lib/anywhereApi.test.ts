import { afterEach, describe, expect, it, vi } from "vitest";

import {
  ANYWHERE_OFFLINE_MESSAGE,
  AnywhereApiError,
  anywhereRequest,
  describeAnywhereError,
  idempotencyKey,
  isAnywhereNetworkError,
  isAnywhereSessionInvalid,
  observeAnywhereUnauthorized,
} from "./anywhereApi";

describe("Anywhere control API", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("keeps access tokens in the authorization header", async () => {
    const fetchMock = vi.fn(async (url: string, init?: RequestInit) => {
      expect(init?.headers).toMatchObject({ authorization: "Bearer secret-access-token" });
      expect(url).not.toContain("secret-access-token");
      expect(String(init?.body ?? "")).not.toContain("secret-access-token");
      return new Response(JSON.stringify({ version: 1 }), { status: 200, headers: { "content-type": "application/json" } });
    });
    vi.stubGlobal("fetch", fetchMock);
    await expect(anywhereRequest("https://app.test", "/v1/me", {}, "secret-access-token")).resolves.toEqual({ version: 1 });
  });

  it("treats pending device authorization as an empty successful poll", async () => {
    vi.stubGlobal("fetch", vi.fn(async () => new Response(null, { status: 202 })));
    await expect(anywhereRequest("https://app.test", "/v1/auth/device/poll", { method: "POST", body: "{}" })).resolves.toBeUndefined();
  });

  it("preserves stable service error codes", async () => {
    vi.stubGlobal("fetch", vi.fn(async () => new Response(JSON.stringify({ code: "entitlement_read_only", message: "read only" }), { status: 403 })));
    await expect(anywhereRequest("https://app.test", "/v1/me")).rejects.toEqual(new AnywhereApiError(403, "entitlement_read_only", "read only"));
  });

  it("reads the service's versioned error envelope", async () => {
    vi.stubGlobal("fetch", vi.fn(async () => new Response(JSON.stringify({
      version: 1,
      error: { code: "device_enrollment_required", message: "approve this device" },
    }), { status: 403 })));

    await expect(anywhereRequest("https://app.test", "/v1/devices", {}, "access"))
      .rejects.toEqual(new AnywhereApiError(403, "device_enrollment_required", "approve this device"));
  });

  it("preserves the server backoff budget on a 429", async () => {
    vi.stubGlobal("fetch", vi.fn(async () => new Response(JSON.stringify({ code: "rate_limited" }), {
      status: 429,
      headers: { "retry-after": "17", "content-type": "application/json" },
    })));

    await expect(anywhereRequest("https://app.test", "/v1/pairings", {}, "access"))
      .rejects.toMatchObject({ status: 429, retryAfterMs: 17_000 });
  });

  it("classifies only authenticated 401 responses as an invalid secure session", async () => {
    expect(isAnywhereSessionInvalid(new AnywhereApiError(401, "invalid_token", "expired"))).toBe(true);
    expect(isAnywhereSessionInvalid(new AnywhereApiError(429, "rate_limited", "later"))).toBe(false);
    expect(isAnywhereSessionInvalid(new AnywhereApiError(503, "dependency_unavailable", "offline"))).toBe(false);
    expect(isAnywhereSessionInvalid(new Error("network unavailable"))).toBe(false);
  });

  it("reports the rejected credential without treating public 401s as session loss", async () => {
    const rejected = vi.fn();
    const stop = observeAnywhereUnauthorized(rejected);
    vi.stubGlobal("fetch", vi.fn(async () => new Response(JSON.stringify({ code: "invalid_token" }), { status: 401 })));

    await expect(anywhereRequest("https://app.test", "/v1/me", {}, "stale-access"))
      .rejects.toMatchObject({ status: 401 });
    await expect(anywhereRequest("https://app.test", "/v1/auth/public"))
      .rejects.toMatchObject({ status: 401 });

    expect(rejected).toHaveBeenCalledOnce();
    expect(rejected).toHaveBeenCalledWith("stale-access");
    stop();
  });

  it("creates visible ASCII idempotency keys with sufficient entropy", () => {
    expect(idempotencyKey()).toMatch(/^[0-9a-f]{32}$/);
    expect(idempotencyKey()).not.toBe(idempotencyKey());
  });
});

describe("Anywhere network-failure classification", () => {
  it("never treats a real service response as a network failure, even a 5xx", () => {
    expect(isAnywhereNetworkError(new AnywhereApiError(503, "dependency_unavailable", "offline"))).toBe(false);
    expect(describeAnywhereError(new AnywhereApiError(403, "access_denied", "access denied"))).toBe("access denied");
  });

  it("classifies the raw Android fetch-layer exception seen offline as a network failure", () => {
    // Exact text observed on-device: `fetch failed: java.net.UnknownHostException: Unable to
    // resolve host "app.forge.adulari.dev": No address associated with hostname`.
    const raw = new Error(
      'fetch failed: java.net.UnknownHostException: Unable to resolve host "app.forge.adulari.dev": No address associated with hostname',
    );
    expect(isAnywhereNetworkError(raw)).toBe(true);
    expect(describeAnywhereError(raw)).toBe(ANYWHERE_OFFLINE_MESSAGE);
  });

  it("classifies other platforms' network exceptions the same way", () => {
    expect(isAnywhereNetworkError(new TypeError("Network request failed"))).toBe(true);
    expect(isAnywhereNetworkError(new TypeError("Failed to fetch"))).toBe(true);
  });

  it("does not misclassify a plain hardcoded validation error as offline", () => {
    // preflightAnywhere throws plain `Error`s for version/readiness checks that already read
    // fine to a person — those must pass through untouched, not get overwritten by the offline
    // message just because they aren't an AnywhereApiError.
    const versionError = new Error("Update Forge before setting up Forge Anywhere.");
    expect(isAnywhereNetworkError(versionError)).toBe(false);
    expect(describeAnywhereError(versionError)).toBe("Update Forge before setting up Forge Anywhere.");
  });
});
