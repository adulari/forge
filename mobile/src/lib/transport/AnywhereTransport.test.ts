import { describe, expect, it } from "vitest";

import { AnywhereTransport, type AnywhereBridgeRequest, type AnywhereRelay } from "./AnywhereTransport";
import type { RemoteSocket } from "./RemoteTransport";

function socket(): RemoteSocket {
  return {
    readyState: 1,
    onopen: null,
    onmessage: null,
    onerror: null,
    onclose: null,
    send: () => {},
    close: () => {},
  };
}

describe("AnywhereTransport", () => {
  it("maps an existing daemon endpoint to a typed bridge route", async () => {
    const captured: AnywhereBridgeRequest[] = [];
    const relay: AnywhereRelay = {
      request: async (request) => {
        captured.push(request);
        return { status: 200, body: new TextEncoder().encode("[]") };
      },
      openSessionSocket: socket,
    };
    const transport = new AnywhereTransport("host-1", relay);
    const response = await transport.fetch("fany://host-1/api/sessions");
    expect(response.status).toBe(200);
    expect(captured[0]?.route).toBe("list_sessions");
  });

  it("refuses arbitrary URLs instead of acting as a proxy", async () => {
    const relay: AnywhereRelay = {
      request: async () => ({ status: 200, body: new Uint8Array() }),
      openSessionSocket: socket,
    };
    const transport = new AnywhereTransport("host-1", relay);
    await expect(transport.fetch("fany://host-1/api/proxy?url=https://example.com"))
      .rejects.toThrow("not allowlisted");
  });

  it("opens only a typed session WebSocket", () => {
    let request: { hostId: string; sessionId: string; revision: number } | null = null;
    const relay: AnywhereRelay = {
      request: async () => ({ status: 200, body: new Uint8Array() }),
      openSessionSocket: (value) => {
        request = value;
        return socket();
      },
    };
    const transport = new AnywhereTransport("host-1", relay);
    transport.openWebSocket("fany-ws://host-1/ws?session=session-7&rev=12");
    expect(request).toEqual({ hostId: "host-1", sessionId: "session-7", revision: 12 });
    expect(() => transport.openWebSocket("fany-ws://host-1/admin"))
      .toThrow("only permits");
  });
});
