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

/**
 * React Native's FormData is a real `FormData` whose entries are not `Blob`s — parts are the
 * plain `{bytes,name,type}` adapters the recorder and file picker append. Subclassing keeps the
 * `instanceof` identity the production code branches on while reproducing those entry values.
 */
class NativeFormData extends FormData {
  private readonly parts: [string, unknown][] = [];

  appendPart(name: string, value: unknown): void {
    this.parts.push([name, value]);
  }

  entries(): FormDataIterator<[string, FormDataEntryValue]> {
    return this.parts[Symbol.iterator]() as unknown as FormDataIterator<[string, FormDataEntryValue]>;
  }
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

  it("relays a FormData body as real multipart bytes with a matching boundary", async () => {
    const captured: AnywhereBridgeRequest[] = [];
    const relay: AnywhereRelay = {
      request: async (request) => {
        captured.push(request);
        return { status: 200, body: new TextEncoder().encode('{"text":"hello"}') };
      },
      openSessionSocket: socket,
    };
    const transport = new AnywhereTransport("host-1", relay);
    // Faithful to native: a real FormData whose entries are the `{bytes,name,type}` adapters
    // voice.ts/attach.ts append. React Native's own `Request` cannot serialise that at all.
    const form = new NativeFormData();
    form.appendPart("file", {
      bytes: async () => new TextEncoder().encode("RIFFdata"),
      name: "voice.wav",
      type: "audio/wav",
    });

    await transport.fetch("fany://host-1/api/voice/transcribe?language=en", {
      method: "POST",
      body: form,
      headers: { Accept: "application/json" },
    });

    const request = captured[0];
    expect(request?.route).toBe("voice_transcribe");
    const contentType = request?.headers.find(([name]) => name === "content-type")?.[1];
    expect(contentType).toMatch(/^multipart\/form-data; boundary=/);
    expect(request?.headers.find(([name]) => name === "accept")?.[1]).toBe("application/json");
    // The relayed bytes are the multipart the declared boundary describes — the daemon reads the
    // audio format hint off that filename (requestBody.test.ts parses one back with a real parser).
    const boundary = contentType?.replace("multipart/form-data; boundary=", "");
    expect(new TextDecoder().decode(request.body)).toBe(
      `--${boundary}\r\n`
        + 'Content-Disposition: form-data; name="file"; filename="voice.wav"\r\n'
        + "Content-Type: audio/wav\r\n\r\n"
        + "RIFFdata\r\n"
        + `--${boundary}--\r\n`,
    );
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

  it("hands the caller's abort signal to the relay", async () => {
    // Without this the relay imposed a deadline of its own on every route, so the 120s budget
    // `transcribeAudio` asks for became 30s — and the host transcribes the whole clip before it
    // answers, which is why long voice memos always came back as a relay timeout.
    const captured: AnywhereBridgeRequest[] = [];
    const relay: AnywhereRelay = {
      request: async (request) => {
        captured.push(request);
        return { status: 200, body: new Uint8Array() };
      },
      openSessionSocket: socket,
    };
    const transport = new AnywhereTransport("host-1", relay);
    const controller = new AbortController();
    await transport.fetch("fany://host-1/api/sessions", { signal: controller.signal });
    expect(captured[0]?.signal).toBe(controller.signal);
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

  it("maps terminal metadata and a validated terminal socket to typed relay routes", async () => {
    const bridgeRequests: AnywhereBridgeRequest[] = [];
    let terminalRequest: Parameters<NonNullable<AnywhereRelay["openTerminalSocket"]>>[0] | null =
      null;
    const relay: AnywhereRelay = {
      request: async (request) => {
        bridgeRequests.push(request);
        return { status: 200, body: new TextEncoder().encode("[]") };
      },
      openSessionSocket: socket,
      openTerminalSocket: (request) => {
        terminalRequest = request;
        return socket();
      },
    };
    const transport = new AnywhereTransport("host-1", relay);

    await transport.fetch("fany://host-1/api/terminals?session=session-7");
    transport.openWebSocket(
      "fany-ws://host-1/ws/terminal?session=session-7&terminal=term-3&cols=120&rows=42&restart=true",
    );

    expect(bridgeRequests[0]).toMatchObject({
      route: "list_terminals",
      parameters: ["?session=session-7"],
    });
    expect(terminalRequest).toEqual({
      hostId: "host-1",
      sessionId: "session-7",
      terminalId: "term-3",
      cols: 120,
      rows: 42,
      restart: true,
    });
    expect(() =>
      transport.openWebSocket(
        "fany-ws://host-1/ws/terminal?session=session-7&terminal=bad%2Fid&cols=80&rows=24",
      ),
    ).toThrow("invalid Forge Anywhere terminal stream parameters");
  });
});
