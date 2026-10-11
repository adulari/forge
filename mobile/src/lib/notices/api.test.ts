import { beforeEach, describe, expect, it, vi } from "vitest";

import { getNotices } from "../api";

const tFetch = vi.hoisted(() => vi.fn());
vi.mock("../transport", () => ({ tFetch, TWebSocket: class {} }));

function ok(body: unknown) {
  return new Response(JSON.stringify(body), { status: 200, headers: { "content-type": "application/json" } });
}

describe("getNotices", () => {
  beforeEach(() => tFetch.mockReset());

  it("asks the daemon for notices after the cursor and maps the response", async () => {
    tFetch.mockResolvedValue(ok({ notices: [{ seq: 4, at: 10, title: "Done", message: "built", priority: "normal", session_id: "s9" }], latest: 4 }));
    const result = await getNotices("https://host/tok", 3);
    expect(String(tFetch.mock.calls[0][0])).toBe("https://host/tok/api/notices?after=3");
    expect(result.notices[0]).toMatchObject({ seq: 4, title: "Done", session_id: "s9" });
    expect(result.latest).toBe(4);
  });

  it("uses the same path over an Anywhere host, which the bridge allowlists as list_notices", async () => {
    tFetch.mockResolvedValue(ok({ notices: [], latest: 0 }));
    await getNotices("fany://host-1");
    expect(String(tFetch.mock.calls[0][0])).toBe("fany://host-1/api/notices?after=0");
  });

  it("clamps a bad cursor", async () => {
    tFetch.mockResolvedValue(ok({ notices: [], latest: 0 }));
    await getNotices("https://host/tok", -5);
    expect(String(tFetch.mock.calls[0][0])).toContain("after=0");
  });
});
