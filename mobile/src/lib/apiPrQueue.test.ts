import { beforeEach, describe, expect, it, vi } from "vitest";

import { getPrQueue, postPrQueueAction } from "./apiPr";

const transport = vi.hoisted(() => ({ tFetch: vi.fn() }));

vi.mock("./transport", () => ({ tFetch: transport.tFetch, TWebSocket: class {} }));

const reply = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });

describe("merge queue api", () => {
  beforeEach(() => transport.tFetch.mockReset());

  it("reads the queue", async () => {
    transport.tFetch.mockResolvedValue(reply({ queues: [] }));
    await expect(getPrQueue("https://forge.test/tok")).resolves.toEqual({ queues: [] });
    expect(transport.tFetch.mock.calls[0][0]).toBe("https://forge.test/tok/api/pr/queue");
  });

  it("posts an action with the session and verb", async () => {
    transport.tFetch.mockResolvedValue(reply({ ok: true, message: "held", queues: [] }));
    const result = await postPrQueueAction("https://forge.test/tok", { session: "s1", action: "hold" });
    const [url, init] = transport.tFetch.mock.calls[0];
    expect(url).toBe("https://forge.test/tok/api/pr/queue/action");
    expect(init.method).toBe("POST");
    expect(JSON.parse(init.body)).toEqual({ session: "s1", action: "hold" });
    expect(result.message).toBe("held");
  });

  it("surfaces the daemon's error text on a conflict", async () => {
    transport.tFetch.mockResolvedValue(reply({ error: "not queued" }, 409));
    await expect(postPrQueueAction("https://forge.test/tok", { session: "s1", action: "front" })).rejects.toThrow("not queued");
  });
});
