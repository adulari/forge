import { describe, expect, it } from "vitest";

import {
  AnywhereTransport,
  responseBodyInit,
  type AnywhereBridgeRequest,
  type AnywhereRelay,
} from "./AnywhereTransport";
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

  it("maps diagnostics to its read-only typed bridge route", async () => {
    const captured: AnywhereBridgeRequest[] = [];
    const relay: AnywhereRelay = {
      request: async (request) => {
        captured.push(request);
        return { status: 200, body: new TextEncoder().encode("{}") };
      },
      openSessionSocket: socket,
    };
    const transport = new AnywhereTransport("host-1", relay);

    await transport.fetch("fany://host-1/api/diagnostics");

    expect(captured[0]).toMatchObject({
      route: "diagnostics",
      method: "GET",
      parameters: [],
    });
    await expect(
      transport.fetch("fany://host-1/api/diagnostics", { method: "POST" }),
    ).rejects.toThrow("not allowlisted");
  });

  it("maps the MCP catalog read but never MCP server registration", async () => {
    // Registering a stdio server persists a command the host executes, so it is a local-only
    // action: the host has no bridge route for it and the phone must not emit one.
    const captured: AnywhereBridgeRequest[] = [];
    const relay: AnywhereRelay = {
      request: async (request) => {
        captured.push(request);
        return { status: 200, body: new TextEncoder().encode("{}") };
      },
      openSessionSocket: socket,
    };
    const transport = new AnywhereTransport("host-1", relay);

    await transport.fetch("fany://host-1/api/mcp");

    expect(captured[0]).toMatchObject({ route: "read_mcp", method: "GET", parameters: [] });
    await expect(
      transport.fetch("fany://host-1/api/mcp", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ name: "evil", transport: "stdio", command: "sh" }),
      }),
    ).rejects.toThrow("not allowlisted");
    await expect(
      transport.fetch("fany://host-1/api/mcp", { method: "PATCH" }),
    ).rejects.toThrow("not allowlisted");
    expect(captured).toHaveLength(1);
  });

  it("maps global search and session lifecycle requests without widening the allowlist", async () => {
    const captured: AnywhereBridgeRequest[] = [];
    const relay: AnywhereRelay = {
      request: async (request) => {
        captured.push(request);
        return { status: 200, body: new TextEncoder().encode("{}") };
      },
      openSessionSocket: socket,
    };
    const transport = new AnywhereTransport("host-1", relay);

    await transport.fetch("fany://host-1/api/sessions/search?q=needle&limit=30");
    await transport.fetch("fany://host-1/api/sessions/session_7", {
      method: "PATCH",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ title: "Renamed" }),
    });
    await transport.fetch("fany://host-1/api/sessions/session_7", { method: "DELETE" });

    expect(captured.map(({ route, method, parameters }) => ({ route, method, parameters }))).toEqual([
      {
        route: "search_sessions",
        method: "GET",
        parameters: ["?q=needle&limit=30"],
      },
      {
        route: "rename_session",
        method: "PATCH",
        parameters: ["session_7"],
      },
      {
        route: "delete_session",
        method: "DELETE",
        parameters: ["session_7"],
      },
    ]);
    await expect(
      transport.fetch("fany://host-1/api/sessions/unsafe%2Fid", { method: "DELETE" }),
    ).rejects.toThrow("invalid Forge Anywhere session path parameter");
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

describe("responseBodyInit", () => {
  const bytes = new TextEncoder().encode(JSON.stringify([{ content: "Resuming — patch ✓" }]));

  it("decodes a JSON body as UTF-8 so non-ASCII text survives the relay", async () => {
    const body = responseBodyInit(bytes, [["Content-Type", "application/json"]]);
    expect(typeof body).toBe("string");
    const parsed = (await new Response(body).json()) as { content: string }[];
    expect(parsed[0].content).toBe("Resuming — patch ✓");
  });

  it("keeps binary bodies as bytes", () => {
    const png = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0xff, 0xfe]);
    expect(responseBodyInit(png, [["content-type", "image/png"]])).toBe(png);
  });

  it("treats an untyped body as text only when it is valid UTF-8", () => {
    expect(responseBodyInit(bytes)).toBe(new TextDecoder().decode(bytes));
    const invalid = new Uint8Array([0xff, 0xfe, 0x00]);
    expect(responseBodyInit(invalid)).toBe(invalid);
  });
});

describe("responseBodyInit under React Native's fetch polyfill", () => {
  it("fixes the Latin-1 decode whatwg-fetch applies to byte bodies", async () => {
    // @ts-expect-error whatwg-fetch ships no type declarations
    const polyfill = (await import("whatwg-fetch")) as { Response: typeof Response };
    const RnResponse = polyfill.Response;
    const bytes = new TextEncoder().encode("Resuming — patch");
    // The defect, reproduced: a byte body comes back one char per byte.
    expect(await new RnResponse(bytes).text()).not.toBe("Resuming — patch");
    const body = responseBodyInit(bytes, [["content-type", "application/json"]]);
    expect(await new RnResponse(body).text()).toBe("Resuming — patch");
  });
});
