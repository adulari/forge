import { ed25519 } from "@noble/curves/ed25519.js";
import { afterEach, describe, expect, it, vi } from "vitest";

import { EncryptedAnywhereRelay, type AnywhereRelayCredentials } from "./EncryptedAnywhereRelay";
import { openEnvelope, sealEnvelope } from "./anywhereEnvelope";

const originalFetch = globalThis.fetch;
const originalWebSocket = globalThis.WebSocket;
const hex = (bytes: Uint8Array) => Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");

afterEach(() => {
  globalThis.fetch = originalFetch;
  globalThis.WebSocket = originalWebSocket;
  vi.restoreAllMocks();
});

function setup() {
  const accountId = new Uint8Array(16).fill(0x11);
  const controllerId = new Uint8Array(16).fill(0x22);
  const hostDeviceId = new Uint8Array(16).fill(0x44);
  const dataKey = new Uint8Array(32).fill(0x55);
  const controllerSeed = new Uint8Array(32).fill(0x66);
  const hostSeed = new Uint8Array(32).fill(0x77);
  let sequence = 0n;
  let hostSequence = 100n;
  const credentials: AnywhereRelayCredentials = {
    serviceUrl: "https://app.forge.test",
    accountId,
    deviceId: controllerId,
    dataKey,
    keyEpoch: 1,
    signingPrivateKey: controllerSeed,
    accessToken: async () => "token",
    reserveSequence: async () => sequence++,
    acceptSequences: async () => true,
    signingPublicKey: async () => ed25519.getPublicKey(hostSeed),
    randomBytes: (length) => {
      const bytes = new Uint8Array(length);
      for (let i = 0; i < length; i += 1) bytes[i] = Math.floor(Math.random() * 256);
      return bytes;
    },
  };
  const recipients: string[] = [];
  const sockets: MockWebSocket[] = [];
  class MockWebSocket {
    static readonly CONNECTING = 0;
    static readonly OPEN = 1;
    static readonly CLOSING = 2;
    static readonly CLOSED = 3;
    readyState = MockWebSocket.CONNECTING;
    binaryType = "blob";
    onopen: (() => void) | null = null;
    onmessage: ((event: { data: ArrayBuffer }) => void) | null = null;
    onerror: (() => void) | null = null;
    onclose: (() => void) | null = null;
    constructor() {
      sockets.push(this);
      queueMicrotask(() => {
        this.readyState = MockWebSocket.OPEN;
        this.onopen?.();
      });
    }
    send(value: Uint8Array): void {
      const request = openEnvelope(value, dataKey, ed25519.getPublicKey(controllerSeed));
      recipients.push(hex(request.metadata.recipientId));
      const payload = JSON.parse(new TextDecoder().decode(request.plaintext)) as { request_id: number[] };
      const response = sealEnvelope(
        {
          kind: 2, flags: 0, accountId, senderDeviceId: hostDeviceId, recipientKind: 1,
          recipientId: controllerId, keyEpoch: 1, sequence: hostSequence++, createdAtMs: 1n,
          nonce: new Uint8Array(24).fill(0x99),
        },
        new TextEncoder().encode(JSON.stringify({
          request_id: payload.request_id, status: 200, headers: [], body: Array.from(request.metadata.recipientId),
        })),
        dataKey,
        hostSeed,
      );
      queueMicrotask(() => this.onmessage?.({ data: response.buffer as ArrayBuffer }));
    }
    close(): void {
      this.readyState = MockWebSocket.CLOSED;
      this.onclose?.();
    }
  }
  const fetchMock = vi.fn(async () => new Response(JSON.stringify({ version: 1, ticket: "t" }), { status: 200 }));
  globalThis.fetch = fetchMock as unknown as typeof fetch;
  globalThis.WebSocket = MockWebSocket as unknown as typeof WebSocket;
  return { credentials, recipients, sockets, fetchMock };
}

const call = (relay: EncryptedAnywhereRelay, hostId: string) => relay.request({
  hostId, route: "list_sessions", parameters: [""], method: "GET", headers: [], body: new Uint8Array(),
});

describe("one relay shared by several hosts", () => {
  const hostA = hex(new Uint8Array(16).fill(0xa1));
  const hostB = hex(new Uint8Array(16).fill(0xb2));

  it("serves every host over a single ticket and socket, addressing each envelope by host", async () => {
    const { credentials, recipients, sockets, fetchMock } = setup();
    const relay = new EncryptedAnywhereRelay(credentials);
    const [a, b] = await Promise.all([call(relay, hostA), call(relay, hostB)]);
    expect(hex(a.body)).toBe(hostA);
    expect(hex(b.body)).toBe(hostB);
    expect(recipients.sort()).toEqual([hostA, hostB].sort());
    expect(fetchMock).toHaveBeenCalledOnce();
    expect(sockets).toHaveLength(1);
  });

  it("warm() opens the connection before the first request", async () => {
    const { credentials, sockets, fetchMock } = setup();
    const relay = new EncryptedAnywhereRelay(credentials);
    await relay.warm();
    expect(sockets).toHaveLength(1);
    await call(relay, hostA);
    expect(fetchMock).toHaveBeenCalledOnce();
  });

  it("close() releases the socket without breaking a later reconnect", async () => {
    const { credentials, sockets } = setup();
    const relay = new EncryptedAnywhereRelay(credentials);
    await call(relay, hostA);
    relay.close();
    expect(sockets[0].readyState).toBe(3);
    const again = await call(relay, hostB);
    expect(hex(again.body)).toBe(hostB);
    expect(sockets).toHaveLength(2);
    expect(sockets[1].readyState).toBe(1);
  });
});
