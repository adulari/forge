import { ed25519 } from "@noble/curves/ed25519.js";
import { afterEach, describe, expect, it, vi } from "vitest";

import { EncryptedAnywhereRelay, type AnywhereRelayCredentials } from "./EncryptedAnywhereRelay";
import { openEnvelope, sealEnvelope } from "./anywhereEnvelope";
import { decodeWireBytes, encodeBase64Url } from "./anywhereWire";

const originalFetch = globalThis.fetch;
const originalWebSocket = globalThis.WebSocket;

afterEach(() => {
  globalThis.fetch = originalFetch;
  globalThis.WebSocket = originalWebSocket;
  vi.restoreAllMocks();
});

const accountId = new Uint8Array(16).fill(0x11);
const controllerId = new Uint8Array(16).fill(0x22);
const hostId = new Uint8Array(16).fill(0x33);
const hostDeviceId = new Uint8Array(16).fill(0x44);
const dataKey = new Uint8Array(32).fill(0x55);
const controllerSeed = new Uint8Array(32).fill(0x66);
const hostSeed = new Uint8Array(32).fill(0x77);
const hostHex = Array.from(hostId, (byte) => byte.toString(16).padStart(2, "0")).join("");

type Seen = Record<string, unknown>;

/** A host stand-in. `advertise` is what a new host does; an old host (and prod today) does not. */
function installHost(advertise: boolean, seen: Seen[]): void {
  let sequence = 100n;
  let counter = 0;
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
      const payload = JSON.parse(new TextDecoder().decode(request.plaintext)) as Seen;
      seen.push(payload);
      // The host decodes whichever form arrived, exactly like the Rust connector.
      const requestId = decodeWireBytes("request id", payload.request_id);
      const v2 = advertise && payload.wire === 2;
      const body = new TextEncoder().encode(JSON.stringify({ echo: counter++ }));
      const answer: Record<string, unknown> = v2
        ? { request_id: encodeBase64Url(requestId), status: 200, headers: [], body: encodeBase64Url(body), wire: 2 }
        : { request_id: Array.from(requestId), status: 200, headers: [], body: Array.from(body) };
      const response = sealEnvelope(
        {
          kind: 2,
          flags: 0,
          accountId,
          senderDeviceId: hostDeviceId,
          recipientKind: 1,
          recipientId: controllerId,
          keyEpoch: 3,
          sequence: sequence++,
          createdAtMs: 1n,
          nonce: new Uint8Array(24).fill(0x99),
        },
        new TextEncoder().encode(JSON.stringify(answer)),
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
  globalThis.WebSocket = MockWebSocket as unknown as typeof WebSocket;
}

function relay(): EncryptedAnywhereRelay {
  let nextSequence = 2n;
  let random = 0;
  const credentials: AnywhereRelayCredentials = {
    serviceUrl: "https://app.forge.test",
    accountId,
    deviceId: controllerId,
    dataKey,
    keyEpoch: 3,
    signingPrivateKey: controllerSeed,
    accessToken: async () => "access-token",
    reserveSequence: async () => nextSequence++,
    acceptSequences: async () => true,
    signingPublicKey: async () => ed25519.getPublicKey(hostSeed),
    randomBytes: (length) => new Uint8Array(length).fill(++random & 0xff),
  };
  return new EncryptedAnywhereRelay(credentials);
}

const request = (body: Uint8Array) => ({
  hostId: hostHex,
  route: "upload" as const,
  parameters: ["session_1"],
  method: "POST",
  headers: [] as [string, string][],
  body,
});

describe("EncryptedAnywhereRelay wire negotiation", () => {
  it("upgrades to v2 only after the host advertises it, and keeps 100 KiB inline", async () => {
    const seen: Seen[] = [];
    installHost(true, seen);
    const anywhere = relay();

    const first = await anywhere.request(request(new Uint8Array([1, 2, 3])));
    expect(JSON.parse(new TextDecoder().decode(first.body))).toEqual({ echo: 0 });
    // The first request cannot know the host yet: v1 arrays, plus the advertisement.
    expect(Array.isArray(seen[0].request_id)).toBe(true);
    expect(seen[0].body).toEqual([1, 2, 3]);
    expect(seen[0].wire).toBe(2);

    const large = new Uint8Array(100 * 1024).fill(7);
    const second = await anywhere.request(request(large));
    expect(JSON.parse(new TextDecoder().decode(second.body))).toEqual({ echo: 1 });
    expect(typeof seen[1].request_id).toBe("string");
    expect(typeof seen[1].body).toBe("string");
    expect(decodeWireBytes("body", seen[1].body)).toEqual(large);
    expect(seen[1].body_blob).toBeUndefined();
    // Only the relay ticket was fetched: no blob reservation for a body v2 carries inline.
    expect(globalThis.fetch).toHaveBeenCalledOnce();
  });

  it("never sends v2 to a host that does not advertise it (an older daemon)", async () => {
    const seen: Seen[] = [];
    installHost(false, seen);
    const anywhere = relay();

    await anywhere.request(request(new Uint8Array([1])));
    const second = await anywhere.request(request(new Uint8Array([4, 5])));
    expect(JSON.parse(new TextDecoder().decode(second.body))).toEqual({ echo: 1 });
    for (const payload of seen) {
      expect(Array.isArray(payload.request_id)).toBe(true);
      expect(Array.isArray(payload.body)).toBe(true);
    }
    expect(seen[1].body).toEqual([4, 5]);
  });
});
