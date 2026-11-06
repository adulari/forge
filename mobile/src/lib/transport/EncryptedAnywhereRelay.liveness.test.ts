import { ed25519 } from "@noble/curves/ed25519.js";
import { afterEach, describe, expect, it, vi } from "vitest";

import { EncryptedAnywhereRelay, type AnywhereRelayCredentials } from "./EncryptedAnywhereRelay";
import { openEnvelope, sealEnvelope } from "./anywhereEnvelope";

const originalFetch = globalThis.fetch;
const originalWebSocket = globalThis.WebSocket;

afterEach(() => {
  vi.useRealTimers();
  globalThis.fetch = originalFetch;
  globalThis.WebSocket = originalWebSocket;
  vi.restoreAllMocks();
});

const accountId = new Uint8Array(16).fill(0x11);
const controllerId = new Uint8Array(16).fill(0x22);
const hostDeviceId = new Uint8Array(16).fill(0x44);
const dataKey = new Uint8Array(32).fill(0x55);
const controllerSeed = new Uint8Array(32).fill(0x66);
const hostSeed = new Uint8Array(32).fill(0x77);

function makeRelay(): EncryptedAnywhereRelay {
  let sequence = 1n;
  const credentials: AnywhereRelayCredentials = {
    serviceUrl: "https://app.forge.test", accountId, deviceId: controllerId, dataKey, keyEpoch: 3,
    signingPrivateKey: controllerSeed, accessToken: async () => "token",
    reserveSequence: async () => sequence++, acceptSequences: async () => true,
    signingPublicKey: async () => ed25519.getPublicKey(hostSeed),
    randomBytes: (length) => new Uint8Array(length).fill(Number(sequence)),
  };
  return new EncryptedAnywhereRelay(credentials);
}

const request = {
  hostId: "33".repeat(16), route: "list_sessions" as const, parameters: [""], method: "GET",
  headers: [], body: new Uint8Array(),
};

const ticketResponse = () => new Response(JSON.stringify({ ticket: "ticket" }), { status: 200 });

describe("EncryptedAnywhereRelay liveness", () => {
  it("gives up on a ticket fetch that never resolves, then reconnects on the next request", async () => {
    vi.useFakeTimers();
    let calls = 0;
    let aborted = false;
    globalThis.fetch = vi.fn((_url: unknown, init?: RequestInit) => {
      calls += 1;
      if (calls === 1) {
        init?.signal?.addEventListener("abort", () => { aborted = true; });
        return new Promise<Response>(() => {});
      }
      return Promise.resolve(ticketResponse());
    }) as typeof fetch;
    class OpenSocket {
      static readonly CONNECTING = 0; static readonly OPEN = 1; static readonly CLOSING = 2; static readonly CLOSED = 3;
      readyState = 0;
      binaryType = "blob";
      onopen: (() => void) | null = null;
      onmessage: ((event: { data: ArrayBuffer }) => void) | null = null;
      onerror: (() => void) | null = null;
      onclose: (() => void) | null = null;
      constructor(readonly url: string) { queueMicrotask(() => { this.readyState = 1; this.onopen?.(); }); }
      send(value: Uint8Array): void {
        const opened = openEnvelope(value, dataKey, ed25519.getPublicKey(controllerSeed));
        const payload = JSON.parse(new TextDecoder().decode(opened.plaintext)) as { request_id: number[] };
        const response = sealEnvelope({
          kind: 2, flags: 0, accountId, senderDeviceId: hostDeviceId, recipientKind: 1,
          recipientId: controllerId, keyEpoch: 3, sequence: 20n, createdAtMs: 1n,
          nonce: new Uint8Array(24).fill(9),
        }, new TextEncoder().encode(JSON.stringify({ request_id: payload.request_id, status: 204, body: [] })), dataKey, hostSeed);
        queueMicrotask(() => this.onmessage?.({ data: response.buffer as ArrayBuffer }));
      }
      close(): void { this.readyState = 3; this.onclose?.(); }
    }
    globalThis.WebSocket = OpenSocket as unknown as typeof WebSocket;

    const relay = makeRelay();
    const first = relay.request(request);
    const firstResult = expect(first).rejects.toThrow("connection timed out");
    await vi.advanceTimersByTimeAsync(15_000);
    await firstResult;
    expect(aborted).toBe(true);
    await expect(relay.request(request)).resolves.toMatchObject({ status: 204 });
    expect(calls).toBe(2);
  });

  it("gives up on a WebSocket that never opens and closes it", async () => {
    vi.useFakeTimers();
    globalThis.fetch = vi.fn(async () => ticketResponse()) as typeof fetch;
    const sockets: { closed: boolean }[] = [];
    class HangingSocket {
      static readonly CONNECTING = 0; static readonly OPEN = 1; static readonly CLOSING = 2; static readonly CLOSED = 3;
      readyState = 0;
      binaryType = "blob";
      closed = false;
      onopen: (() => void) | null = null;
      onmessage: ((event: { data: ArrayBuffer }) => void) | null = null;
      onerror: (() => void) | null = null;
      onclose: (() => void) | null = null;
      constructor(readonly url: string) { sockets.push(this); }
      send(): void {}
      close(): void { this.closed = true; this.readyState = 3; }
    }
    globalThis.WebSocket = HangingSocket as unknown as typeof WebSocket;

    const relay = makeRelay();
    const pending = expect(relay.request(request)).rejects.toThrow("connection timed out");
    await vi.advanceTimersByTimeAsync(15_000);
    await pending;
    expect(sockets).toHaveLength(1);
    expect(sockets[0].closed).toBe(true);
  });

  it("reconnects on a new socket after a request times out on a silent one", async () => {
    vi.useFakeTimers();
    globalThis.fetch = vi.fn(async () => ticketResponse()) as typeof fetch;
    const sockets: SwitchableSocket[] = [];
    class SwitchableSocket {
      static readonly CONNECTING = 0; static readonly OPEN = 1; static readonly CLOSING = 2; static readonly CLOSED = 3;
      readyState = 0;
      binaryType = "blob";
      closed = false;
      onopen: (() => void) | null = null;
      onmessage: ((event: { data: ArrayBuffer }) => void) | null = null;
      onerror: (() => void) | null = null;
      onclose: (() => void) | null = null;
      readonly number: number;
      constructor(readonly url: string) {
        this.number = sockets.push(this);
        queueMicrotask(() => { this.readyState = 1; this.onopen?.(); });
      }
      send(value: Uint8Array): void {
        if (this.number === 1) return;
        const opened = openEnvelope(value, dataKey, ed25519.getPublicKey(controllerSeed));
        const payload = JSON.parse(new TextDecoder().decode(opened.plaintext)) as { request_id: number[] };
        const response = sealEnvelope({
          kind: 2, flags: 0, accountId, senderDeviceId: hostDeviceId, recipientKind: 1,
          recipientId: controllerId, keyEpoch: 3, sequence: 30n, createdAtMs: 1n,
          nonce: new Uint8Array(24).fill(8),
        }, new TextEncoder().encode(JSON.stringify({ request_id: payload.request_id, status: 200, body: [] })), dataKey, hostSeed);
        queueMicrotask(() => this.onmessage?.({ data: response.buffer as ArrayBuffer }));
      }
      close(): void { this.closed = true; this.readyState = 3; this.onclose?.(); }
    }
    globalThis.WebSocket = SwitchableSocket as unknown as typeof WebSocket;

    const relay = makeRelay();
    const first = expect(relay.request(request)).rejects.toThrow("request timed out");
    await vi.advanceTimersByTimeAsync(30_000);
    await first;
    expect(sockets[0].closed).toBe(true);
    await expect(relay.request(request)).resolves.toMatchObject({ status: 200 });
    expect(sockets).toHaveLength(2);
  });

  it("keeps a live socket when a slow request times out after other traffic arrived", async () => {
    vi.useFakeTimers();
    globalThis.fetch = vi.fn(async () => ticketResponse()) as typeof fetch;
    const sockets: ChattySocket[] = [];
    class ChattySocket {
      static readonly CONNECTING = 0; static readonly OPEN = 1; static readonly CLOSING = 2; static readonly CLOSED = 3;
      readyState = 0;
      binaryType = "blob";
      closed = false;
      onopen: (() => void) | null = null;
      onmessage: ((event: { data: ArrayBuffer }) => void) | null = null;
      onerror: (() => void) | null = null;
      onclose: (() => void) | null = null;
      constructor(readonly url: string) {
        sockets.push(this);
        queueMicrotask(() => { this.readyState = 1; this.onopen?.(); });
      }
      send(): void {}
      close(): void { this.closed = true; this.readyState = 3; this.onclose?.(); }
    }
    globalThis.WebSocket = ChattySocket as unknown as typeof WebSocket;

    const relay = makeRelay();
    const slow = expect(relay.request(request)).rejects.toThrow("request timed out");
    await vi.advanceTimersByTimeAsync(10_000);
    // Valid traffic for some other request proves the socket is alive.
    const other = sealEnvelope({
      kind: 2, flags: 0, accountId, senderDeviceId: hostDeviceId, recipientKind: 1,
      recipientId: controllerId, keyEpoch: 3, sequence: 40n, createdAtMs: 1n,
      nonce: new Uint8Array(24).fill(7),
    }, new TextEncoder().encode(JSON.stringify({ request_id: Array(16).fill(99), status: 200, body: [] })), dataKey, hostSeed);
    sockets[0].onmessage?.({ data: other.buffer as ArrayBuffer });
    await vi.advanceTimersByTimeAsync(20_000);
    await slow;
    expect(sockets[0].closed).toBe(false);
  });
});
