import { afterEach, describe, expect, it, vi } from "vitest";

import { AnywhereApiError, anywhereRequest, idempotencyKey } from "./anywhereApi";

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

  it("creates visible ASCII idempotency keys with sufficient entropy", () => {
    expect(idempotencyKey()).toMatch(/^[0-9a-f]{32}$/);
    expect(idempotencyKey()).not.toBe(idempotencyKey());
  });
});
