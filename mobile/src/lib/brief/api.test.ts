import { beforeEach, describe, expect, it, vi } from "vitest";

import { getBrief } from "./api";

const tFetch = vi.hoisted(() => vi.fn());
vi.mock("../transport", () => ({ tFetch, TWebSocket: class {} }));

function ok(body: unknown) {
  return new Response(JSON.stringify(body), { status: 200, headers: { "content-type": "application/json" } });
}

describe("getBrief", () => {
  beforeEach(() => tFetch.mockReset());

  it("asks for the window and maps the digest", async () => {
    tFetch.mockResolvedValue(
      ok({ since: 100, now: 200, totals: { sessions: 1, finished: 1 }, sessions: [{ id: "s1", title: "T", outcome: "finished", summary: "ok" }] }),
    );
    const brief = await getBrief("https://host/tok", 100.9);
    expect(String(tFetch.mock.calls[0][0])).toBe("https://host/tok/api/brief?since=100");
    expect(brief.sessions[0]).toMatchObject({ id: "s1", outcome: "finished" });
  });
});
