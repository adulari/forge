// The Tauri branch of the transport seam. `isTauri` is decided once at module load, so the
// platform flag is mocked rather than simulated through a fake `window`.
import { beforeEach, describe, expect, it, vi } from "vitest";

import { TWebSocket } from "./index";

vi.mock("../platform", () => ({
  isTauri: true,
  isWeb: false,
  isNative: true,
  isIOS: false,
  isMacOS: false,
}));
vi.mock("expo-secure-store", () => ({}));

const plugin = vi.hoisted(() => {
  const listeners: ((message: unknown) => void)[] = [];
  return {
    listeners,
    socket: {
      addListener: (callback: (message: unknown) => void) => {
        listeners.push(callback);
        return () => {};
      },
      send: vi.fn(async () => {}),
      disconnect: vi.fn(async () => {}),
    },
  };
});

vi.mock("@tauri-apps/plugin-websocket", () => ({
  default: { connect: vi.fn(async () => plugin.socket) },
}));

class FakeWebSocket {
  static instances: FakeWebSocket[] = [];
  binaryType = "blob";
  onopen: ((event: Event) => void) | null = null;
  onmessage: ((event: MessageEvent) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;
  onclose: ((event: CloseEvent) => void) | null = null;
  closed = false;

  constructor(readonly url: string) {
    FakeWebSocket.instances.push(this);
  }

  send() {}
  close() {
    this.closed = true;
  }
}

vi.stubGlobal("WebSocket", FakeWebSocket);

/** The shim reaches the plugin through a dynamic `import()`, so the listener lands a few real
 * ticks after the failing native socket closes. */
async function awaitPluginListener(): Promise<(message: unknown) => void> {
  await vi.waitFor(() => expect(plugin.listeners).toHaveLength(1));
  return plugin.listeners[0];
}

beforeEach(() => {
  FakeWebSocket.instances = [];
  plugin.listeners.length = 0;
});

describe("TauriAwareWebSocket binary frames", () => {
  it("asks the real socket for arraybuffer frames by default", () => {
    new TWebSocket("ws://host.local/tok/ws/terminal?session=s1");
    expect(FakeWebSocket.instances[0]?.binaryType).toBe("arraybuffer");
  });

  it("forwards a binaryType assignment to the real socket instead of shadowing it", () => {
    const socket = new TWebSocket("ws://host.local/tok/ws/terminal?session=s1");
    socket.binaryType = "arraybuffer";
    expect(socket.binaryType).toBe("arraybuffer");
    expect(FakeWebSocket.instances[0]?.binaryType).toBe("arraybuffer");

    socket.binaryType = "blob";
    expect(FakeWebSocket.instances[0]?.binaryType).toBe("blob");
  });

  it("delivers pty output over the plugin fallback instead of dropping it", async () => {
    const socket = new TWebSocket("ws://host.local/tok/ws/terminal?session=s1");
    socket.binaryType = "arraybuffer";
    const frames: unknown[] = [];
    socket.onmessage = (event) => frames.push(event.data);

    // A mixed-content block looks like a close before the socket ever opened.
    FakeWebSocket.instances[0]?.onclose?.({} as CloseEvent);
    const listener = await awaitPluginListener();

    listener({ type: "Text", data: "pty spawn failed" });
    listener({ type: "Binary", data: [104, 105] });

    expect(frames[0]).toBe("pty spawn failed");
    expect(frames[1]).toBeInstanceOf(ArrayBuffer);
    expect(new TextDecoder().decode(new Uint8Array(frames[1] as ArrayBuffer))).toBe("hi");
  });

  it("still reports a plugin close", async () => {
    const socket = new TWebSocket("ws://host.local/tok/ws/terminal?session=s1");
    let closed: { code: number; reason: string } | null = null;
    socket.onclose = (event) => {
      closed = { code: event.code, reason: event.reason };
    };

    FakeWebSocket.instances[0]?.onclose?.({} as CloseEvent);
    const listener = await awaitPluginListener();
    listener({ type: "Close", data: { code: 1001, reason: "gone" } });

    expect(closed).toEqual({ code: 1001, reason: "gone" });
  });
});
