import { describe, expect, it, vi } from "vitest";

import { DirectTransport } from "./DirectTransport";

describe("DirectTransport", () => {
  it("continues to pass direct fetch and WebSocket inputs through unchanged", async () => {
    const response = new Response("direct", { status: 201 });
    const directFetch = vi.fn(async () => response) as unknown as typeof fetch;
    const sockets: { url: string; protocols?: string | string[] }[] = [];
    class DirectSocket {
      constructor(url: string, protocols?: string | string[]) {
        sockets.push({ url, protocols });
      }
    }
    const transport = new DirectTransport(directFetch, DirectSocket as unknown as typeof WebSocket);
    const init = { method: "POST", body: "unchanged" };

    await expect(transport.fetch("https://host.test/api", init)).resolves.toBe(response);
    transport.openWebSocket("wss://host.test/ws", ["forge-v8"]);

    expect(directFetch).toHaveBeenCalledWith("https://host.test/api", init);
    expect(sockets).toEqual([{ url: "wss://host.test/ws", protocols: ["forge-v8"] }]);
  });
});
