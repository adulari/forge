import { ed25519 } from "@noble/curves/ed25519.js";
import { afterEach, describe, expect, it, vi } from "vitest";

import { EncryptedAnywhereRelay, type AnywhereRelayCredentials } from "./EncryptedAnywhereRelay";
import { decodeEnvelope, openEnvelope, sealEnvelope } from "./anywhereEnvelope";

const originalFetch = globalThis.fetch;
const originalWebSocket = globalThis.WebSocket;

afterEach(() => {
  globalThis.fetch = originalFetch;
  globalThis.WebSocket = originalWebSocket;
  vi.restoreAllMocks();
});

describe("EncryptedAnywhereRelay", () => {
  it("tickets, encrypts a typed request, and authenticates the host response", async () => {
    const accountId = new Uint8Array(16).fill(0x11);
    const controllerId = new Uint8Array(16).fill(0x22);
    const hostId = new Uint8Array(16).fill(0x33);
    const hostDeviceId = new Uint8Array(16).fill(0x44);
    const dataKey = new Uint8Array(32).fill(0x55);
    const controllerSeed = new Uint8Array(32).fill(0x66);
    const hostSeed = new Uint8Array(32).fill(0x77);
    let nextSequence = 2n;
    const accepted: bigint[] = [];
    const credentials: AnywhereRelayCredentials = {
      serviceUrl: "https://app.forge.test",
      accountId,
      deviceId: controllerId,
      dataKey,
      keyEpoch: 3,
      signingPrivateKey: controllerSeed,
      accessToken: async () => "access-token",
      reserveSequence: async () => nextSequence++,
      acceptSequences: async (_sender, _epoch, sequences) => {
        accepted.push(...sequences);
        return true;
      },
      signingPublicKey: async () => ed25519.getPublicKey(hostSeed),
      randomBytes: (length) => new Uint8Array(length).fill(length),
    };
    globalThis.fetch = vi.fn(async () =>
      new Response(JSON.stringify({ version: 1, ticket: "ticket" }), {
        status: 200,
        headers: { "content-type": "application/json" },
      }),
    ) as typeof fetch;

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

      constructor(readonly url: string) {
        queueMicrotask(() => {
          this.readyState = MockWebSocket.OPEN;
          this.onopen?.();
        });
      }

      send(value: Uint8Array): void {
        const request = openEnvelope(value, dataKey, ed25519.getPublicKey(controllerSeed));
        expect(request.metadata.kind).toBe(1);
        expect(request.metadata.sequence).toBe(2n);
        const payload = JSON.parse(new TextDecoder().decode(request.plaintext)) as {
          request_id: number[];
          route: string;
        };
        expect(payload.route).toBe("list_sessions");
        const response = sealEnvelope(
          {
            kind: 2,
            flags: 0,
            accountId,
            senderDeviceId: hostDeviceId,
            recipientKind: 1,
            recipientId: controllerId,
            keyEpoch: 3,
            sequence: 9n,
            createdAtMs: 1n,
            nonce: new Uint8Array(24).fill(0x99),
          },
          new TextEncoder().encode(JSON.stringify({
            request_id: payload.request_id,
            status: 200,
            headers: [["content-type", "application/json"]],
            body: [91, 93],
          })),
          dataKey,
          hostSeed,
        );
        expect(decodeEnvelope(response).metadata.recipientId).toEqual(controllerId);
        queueMicrotask(() => this.onmessage?.({ data: response.buffer as ArrayBuffer }));
      }

      close(): void {
        this.readyState = MockWebSocket.CLOSED;
        this.onclose?.();
      }
    }
    globalThis.WebSocket = MockWebSocket as unknown as typeof WebSocket;

    const relay = new EncryptedAnywhereRelay(credentials);
    const response = await relay.request({
      hostId: Array.from(hostId, (byte) => byte.toString(16).padStart(2, "0")).join(""),
      route: "list_sessions",
      parameters: [""],
      method: "GET",
      headers: [],
      body: new Uint8Array(),
    });
    expect(response.status).toBe(200);
    expect(new TextDecoder().decode(response.body)).toBe("[]");
    expect(accepted).toEqual([9n]);
    expect(globalThis.fetch).toHaveBeenCalledOnce();
  });

  it("processes valid traffic on a new connection after an invalid frame", async () => {
    const accountId = new Uint8Array(16).fill(0x11);
    const controllerId = new Uint8Array(16).fill(0x22);
    const hostId = new Uint8Array(16).fill(0x33);
    const hostDeviceId = new Uint8Array(16).fill(0x44);
    const dataKey = new Uint8Array(32).fill(0x55);
    const controllerSeed = new Uint8Array(32).fill(0x66);
    const hostSeed = new Uint8Array(32).fill(0x77);
    let sequence = 1n;
    const credentials: AnywhereRelayCredentials = {
      serviceUrl: "https://app.forge.test", accountId, deviceId: controllerId, dataKey, keyEpoch: 3,
      signingPrivateKey: controllerSeed, accessToken: async () => "token",
      reserveSequence: async () => sequence++, acceptSequences: async () => true,
      signingPublicKey: async () => ed25519.getPublicKey(hostSeed),
      randomBytes: (length) => new Uint8Array(length).fill(Number(sequence)),
    };
    globalThis.fetch = vi.fn(async () => new Response(JSON.stringify({ ticket: "ticket" }), { status: 200 })) as typeof fetch;
    let connectionNumber = 0;
    class ReconnectingWebSocket {
      static readonly CONNECTING = 0; static readonly OPEN = 1; static readonly CLOSING = 2; static readonly CLOSED = 3;
      readyState = ReconnectingWebSocket.CONNECTING;
      binaryType = "blob";
      onopen: (() => void) | null = null;
      onmessage: ((event: { data: ArrayBuffer }) => void) | null = null;
      onerror: (() => void) | null = null;
      onclose: (() => void) | null = null;
      readonly number = ++connectionNumber;
      constructor(readonly url: string) { queueMicrotask(() => { this.readyState = ReconnectingWebSocket.OPEN; this.onopen?.(); }); }
      send(value: Uint8Array): void {
        if (this.number === 1) {
          queueMicrotask(() => this.onmessage?.({ data: new Uint8Array([0xff]).buffer }));
          return;
        }
        const request = openEnvelope(value, dataKey, ed25519.getPublicKey(controllerSeed));
        const payload = JSON.parse(new TextDecoder().decode(request.plaintext)) as { request_id: number[] };
        const response = sealEnvelope({
          kind: 2, flags: 0, accountId, senderDeviceId: hostDeviceId, recipientKind: 1,
          recipientId: controllerId, keyEpoch: 3, sequence: 20n, createdAtMs: 1n,
          nonce: new Uint8Array(24).fill(9),
        }, new TextEncoder().encode(JSON.stringify({ request_id: payload.request_id, status: 204, body: [] })), dataKey, hostSeed);
        queueMicrotask(() => this.onmessage?.({ data: response.buffer as ArrayBuffer }));
      }
      close(): void { this.readyState = ReconnectingWebSocket.CLOSED; this.onclose?.(); }
    }
    globalThis.WebSocket = ReconnectingWebSocket as unknown as typeof WebSocket;
    const relay = new EncryptedAnywhereRelay(credentials);
    const request = { hostId: Array.from(hostId, (byte) => byte.toString(16).padStart(2, "0")).join(""), route: "list_sessions" as const, parameters: [""], method: "GET", headers: [], body: new Uint8Array() };
    await expect(relay.request(request)).rejects.toThrow();
    await expect(relay.request(request)).resolves.toMatchObject({ status: 204 });
    expect(connectionNumber).toBe(2);
  });
});
