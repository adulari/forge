import { beforeEach, describe, expect, it, vi } from "vitest";

import { closeTerminal, openTerminalSocket } from "./api";

const transportHarness = vi.hoisted(() => ({
  sockets: [] as unknown[],
}));

vi.mock("./transport", () => {
  class MockWebSocket {
    static readonly CONNECTING = 0;
    static readonly OPEN = 1;
    static readonly CLOSING = 2;
    static readonly CLOSED = 3;
    readyState = MockWebSocket.CONNECTING;
    binaryType = "";
    onopen: (() => void) | null = null;
    onmessage: ((event: MessageEvent) => void) | null = null;
    onerror: ((error: unknown) => void) | null = null;
    onclose: ((event: { code?: number; reason?: string }) => void) | null = null;
    readonly sent: string[] = [];
    closed = false;

    constructor(readonly url: string) {
      transportHarness.sockets.push(this);
    }

    send(value: string): void {
      this.sent.push(value);
    }

    close(): void {
      this.closed = true;
      this.readyState = MockWebSocket.CLOSED;
    }
  }

  return {
    tFetch: vi.fn(),
    TWebSocket: MockWebSocket,
  };
});

interface MockSocket {
  url: string;
  readyState: number;
  sent: string[];
  closed: boolean;
  onopen: (() => void) | null;
  onmessage: ((event: MessageEvent) => void) | null;
}

describe("terminal API", () => {
  beforeEach(() => {
    transportHarness.sockets.length = 0;
  });

  it("attaches by terminal id and preserves control and streaming binary frames", () => {
    const output: string[] = [];
    const statuses: string[] = [];
    let clears = 0;
    const client = openTerminalSocket(
      "https://forge.test/token",
      "session-7",
      {
        onOutput: (chunk) => output.push(chunk),
        onStatus: (status) => statuses.push(status),
        onClear: () => { clears += 1; },
      },
      {
        terminalId: "term-3",
        size: { cols: 120, rows: 42 },
        restart: true,
      },
    );
    const socket = transportHarness.sockets[0] as MockSocket;
    expect(socket.url).toContain(
      "/ws/terminal?session=session-7&terminal=term-3&cols=120&rows=42&restart=true",
    );
    socket.readyState = 1;
    socket.onopen?.();
    socket.onmessage?.({
      data: '{"kind":"status","status":"running"}',
    } as MessageEvent);
    socket.onmessage?.({ data: '{"kind":"cleared"}' } as MessageEvent);
    socket.onmessage?.({ data: new Uint8Array([0xe2, 0x82]).buffer } as MessageEvent);
    socket.onmessage?.({ data: new Uint8Array([0xac]).buffer } as MessageEvent);

    expect(statuses).toEqual(["running"]);
    expect(clears).toBe(1);
    expect(output.join("")).toBe("€");
    expect(client.clear()).toBe(true);
    expect(client.kill()).toBe(true);
    client.close();
    expect(socket.sent.map((frame) => JSON.parse(frame))).toEqual([
      { kind: "clear" },
      { kind: "close" },
    ]);
    expect(socket.closed).toBe(true);
  });

  it("hands the shell's exit code to the status handler and treats a missing one as null", () => {
    const seen: [string, number | null][] = [];
    openTerminalSocket("https://forge.test/token", "session-7", {
      onOutput: () => undefined,
      onStatus: (status, code) => seen.push([status, code]),
    });
    const socket = transportHarness.sockets[0] as MockSocket;
    socket.onmessage?.({ data: '{"kind":"status","status":"running"}' } as MessageEvent);
    socket.onmessage?.({ data: '{"kind":"status","status":"exited","exit_code":3}' } as MessageEvent);
    socket.onmessage?.({ data: '{"kind":"status","status":"exited"}' } as MessageEvent);
    socket.onmessage?.({ data: '{"kind":"status","status":"exited","exit_code":"x"}' } as MessageEvent);
    expect(seen).toEqual([
      ["running", null],
      ["exited", 3],
      ["exited", null],
      ["exited", null],
    ]);
  });

  it("closes a terminal by attaching, sending the close frame, then detaching", () => {
    vi.useFakeTimers();
    closeTerminal("https://forge.test/token", "session-7", "term-2");
    const socket = transportHarness.sockets[0] as MockSocket;
    expect(socket.url).toContain("terminal=term-2");
    socket.readyState = 1;
    socket.onopen?.();
    expect(socket.sent.map((frame) => JSON.parse(frame))).toEqual([{ kind: "close" }]);
    vi.advanceTimersByTime(500);
    expect(socket.closed).toBe(true);
    vi.useRealTimers();
  });
});
