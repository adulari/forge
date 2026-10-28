import { beforeEach, describe, expect, it, vi } from "vitest";

import { ApiError, getGitStatus } from "./api";

const transport = vi.hoisted(() => ({ tFetch: vi.fn() }));

vi.mock("./transport", () => ({ tFetch: transport.tFetch, TWebSocket: class {} }));

describe("404 handling", () => {
  beforeEach(() => transport.tFetch.mockReset());

  it("surfaces the daemon's own error for a known route that does not know the session", async () => {
    transport.tFetch.mockResolvedValue(
      new Response(JSON.stringify({ error: "no such session" }), {
        status: 404,
        headers: { "content-type": "application/json" },
      }),
    );
    const error = await getGitStatus("https://forge.test/tok", "ghost").catch((e: unknown) => e);
    expect(error).toBeInstanceOf(ApiError);
    expect((error as ApiError).status).toBe(404);
    expect((error as ApiError).message).toBe("no such session");
  });

  it("keeps a bodiless 404 (a wrong token) generic so token validity is not revealed", async () => {
    transport.tFetch.mockResolvedValue(new Response("Not Found", { status: 404 }));
    const error = await getGitStatus("https://forge.test/tok", "any").catch((e: unknown) => e);
    expect((error as ApiError).message).toBe("pairing invalid, re-scan");
  });
});
