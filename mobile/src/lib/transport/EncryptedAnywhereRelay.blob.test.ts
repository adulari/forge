import { ed25519 } from "@noble/curves/ed25519.js";
import { sha256 } from "@noble/hashes/sha2.js";
import { afterEach, describe, expect, it, vi } from "vitest";

import { EncryptedAnywhereRelay, type AnywhereRelayCredentials } from "./EncryptedAnywhereRelay";
import { bytesToHex, openEnvelope, sealEnvelope, type EnvelopeMetadata } from "./anywhereEnvelope";

const INLINE_LIMIT = 256 * 1024;
const accountId = new Uint8Array(16).fill(0x11);
const controllerId = new Uint8Array(16).fill(0x22);
const hostId = new Uint8Array(16).fill(0x33);
const hostDeviceId = new Uint8Array(16).fill(0x44);
const alternateHostDeviceId = new Uint8Array(16).fill(0x45);
const dataKey = new Uint8Array(32).fill(0x55);
const controllerSeed = new Uint8Array(32).fill(0x66);
const hostSeed = new Uint8Array(32).fill(0x77);
const hostIdHex = bytesToHex(hostId);
const blobId = "ab".repeat(16);
const originalFetch = globalThis.fetch;
const originalWebSocket = globalThis.WebSocket;

afterEach(() => {
  globalThis.fetch = originalFetch;
  globalThis.WebSocket = originalWebSocket;
  vi.restoreAllMocks();
});

describe("EncryptedAnywhereRelay blobs", () => {
  it("keeps 256 KiB inline and uploads request bytes above the threshold", async () => {
    const sentPayloads: Record<string, unknown>[] = [];
    const uploads: Uint8Array[] = [];
    let hostSequence = 40n;
    installWebSocket((socket, value) => {
      const request = openEnvelope(value, dataKey, ed25519.getPublicKey(controllerSeed));
      expect(request.metadata.kind).toBe(1);
      const payload = JSON.parse(new TextDecoder().decode(request.plaintext)) as Record<string, unknown> & {
        request_id: number[];
      };
      sentPayloads.push(payload);
      socket.emit(hostEnvelope(2, {
        request_id: payload.request_id,
        status: 200,
        body: [],
      }, hostSequence++));
    });
    globalThis.fetch = vi.fn(async (input, init) => {
      const url = input.toString();
      if (url.endsWith("/v1/relay/tickets")) return jsonResponse({ ticket: "ticket" });
      if (url.endsWith("/v1/relay/blobs") && init?.method === "POST") {
        const headers = new Headers(init.headers);
        expect(headers.get("authorization")).toBe("Bearer access-token");
        expect(headers.get("Idempotency-Key")).toMatch(/^[0-9a-f]{32}$/);
        const body = JSON.parse(String(init.body)) as Record<string, unknown>;
        expect(body).toMatchObject({ recipient_kind: "host", recipient_id: hostIdHex });
        return jsonResponse({ blob_id: blobId, upload_url: "https://objects.test/upload" });
      }
      if (url === "https://objects.test/upload" && init?.method === "PUT") {
        const bytes = new Uint8Array(init.body as ArrayBuffer);
        uploads.push(bytes);
        const opened = openEnvelope(bytes, dataKey, ed25519.getPublicKey(controllerSeed));
        expect(opened.metadata.kind).toBe(8);
        expect(opened.metadata.recipientId).toEqual(hostId);
        expect(opened.plaintext.length).toBe(INLINE_LIMIT + 1);
        expect(new Headers(init.headers).has("authorization")).toBe(false);
        return new Response(null, { status: 200 });
      }
      if (url.endsWith(`/v1/relay/blobs/${blobId}/complete`)) {
        expect(new Headers(init?.headers).get("Idempotency-Key")).toMatch(/^[0-9a-f]{32}$/);
        return new Response(null, { status: 204 });
      }
      throw new Error(`unexpected fetch ${url}`);
    }) as typeof fetch;

    const relay = new EncryptedAnywhereRelay(credentials());
    await relay.request(bridgeRequest(new Uint8Array(INLINE_LIMIT)));
    await relay.request(bridgeRequest(new Uint8Array(INLINE_LIMIT + 1).fill(7)));

    expect(sentPayloads[0]?.body).toHaveLength(INLINE_LIMIT);
    expect(sentPayloads[0]).not.toHaveProperty("body_blob");
    expect(sentPayloads[1]).not.toHaveProperty("body");
    expect(sentPayloads[1]?.body_blob).toMatchObject({ blob_id: blobId });
    expect(uploads).toHaveLength(1);
  });

  it("uploads WebSocket data above the threshold and sends only bytes_blob", async () => {
    let resolveFrame: (() => void) | undefined;
    const frameSent = new Promise<void>((resolve) => { resolveFrame = resolve; });
    installWebSocket((socket, value) => {
      const opened = openEnvelope(value, dataKey, ed25519.getPublicKey(controllerSeed));
      const payload = JSON.parse(new TextDecoder().decode(opened.plaintext)) as {
        request_id?: number[];
        bytes?: number[];
        bytes_blob?: { blob_id: string };
      };
      if (opened.metadata.kind === 1) {
        socket.emit(hostEnvelope(2, { request_id: payload.request_id, status: 200, body: [] }, 50n));
      } else {
        expect(opened.metadata.kind).toBe(3);
        expect(payload.bytes).toBeUndefined();
        expect(payload.bytes_blob?.blob_id).toBe(blobId);
        resolveFrame?.();
      }
    });
    globalThis.fetch = blobUploadFetch();
    const relay = new EncryptedAnywhereRelay(credentials());
    const socket = relay.openSessionSocket({ hostId: hostIdHex, sessionId: "session", revision: 1 });
    await new Promise<void>((resolve) => { socket.onopen = () => resolve(); });
    socket.send(new Uint8Array(INLINE_LIMIT + 1));
    await frameSent;
  });

  it("rejects a tampered download without consuming it", async () => {
    const original = inboundBlob(new TextEncoder().encode("secret"));
    const tampered = original.slice();
    tampered[120] ^= 1;
    const consumed: string[] = [];
    const relay = inboundRelay(original, tampered, consumed);
    await expect(relay.request(bridgeRequest(new Uint8Array()))).rejects.toThrow();
    expect(consumed).not.toContain("consume");
  });

  it("rejects blob envelope metadata that does not match the outer sender", async () => {
    const mismatched = inboundBlob(new Uint8Array([1, 2, 3]), {
      senderDeviceId: alternateHostDeviceId,
    });
    const consumed: string[] = [];
    const relay = inboundRelay(mismatched, mismatched, consumed);
    await expect(relay.request(bridgeRequest(new Uint8Array()))).rejects.toThrow("sender");
    expect(consumed).not.toContain("consume");
  });

  it("consumes only after blob and outer metadata authenticate, before delivery", async () => {
    const events: string[] = [];
    const blob = inboundBlob(new TextEncoder().encode("large response"));
    const relay = inboundRelay(blob, blob, events, (sequence) => {
      events.push(`accept:${sequence}`);
    });
    const response = await relay.request(bridgeRequest(new Uint8Array()));
    events.push("delivered");
    expect(new TextDecoder().decode(response.body)).toBe("large response");
    expect(events).toEqual(["claim", "download", "accept:8", "accept:9", "consume", "delivered"]);
  });

  it("delivers an accepted blob when temporary ciphertext cleanup fails", async () => {
    const events: string[] = [];
    const blob = inboundBlob(new TextEncoder().encode("durable response"));
    const relay = inboundRelay(blob, blob, events, undefined, 503);
    const response = await relay.request(bridgeRequest(new Uint8Array()));
    expect(new TextDecoder().decode(response.body)).toBe("durable response");
    expect(events).toContain("consume");
  });
});

function credentials(onAccept?: (sequence: bigint) => void): AnywhereRelayCredentials {
  let sequence = 1n;
  let randomCounter = 1;
  return {
    serviceUrl: "https://app.forge.test",
    accountId,
    deviceId: controllerId,
    dataKey,
    keyEpoch: 3,
    signingPrivateKey: controllerSeed,
    accessToken: async () => "access-token",
    reserveSequence: async () => sequence++,
    acceptSequences: async (_sender, _epoch, accepted) => {
      for (const sequence of accepted) onAccept?.(sequence);
      return true;
    },
    signingPublicKey: async () => ed25519.getPublicKey(hostSeed),
    randomBytes: (length) => {
      const bytes = new Uint8Array(length).fill(randomCounter);
      randomCounter = (randomCounter + 1) & 0xff;
      return bytes;
    },
  };
}

function bridgeRequest(body: Uint8Array) {
  return {
    hostId: hostIdHex,
    route: "session_input" as const,
    parameters: ["session"],
    method: "POST",
    headers: [],
    body,
  };
}

function hostEnvelope(kind: 2 | 3, payload: unknown, sequence: bigint): Uint8Array {
  return sealEnvelope(
    hostMetadata(kind, sequence),
    new TextEncoder().encode(JSON.stringify(payload)),
    dataKey,
    hostSeed,
  );
}

function inboundBlob(plaintext: Uint8Array, overrides: Partial<EnvelopeMetadata> = {}): Uint8Array {
  return sealEnvelope(
    { ...hostMetadata(8, 8n), ...overrides },
    plaintext,
    dataKey,
    hostSeed,
  );
}

function hostMetadata(kind: 2 | 3 | 8, sequence: bigint): EnvelopeMetadata {
  return {
    kind,
    flags: 0,
    accountId,
    senderDeviceId: hostDeviceId,
    recipientKind: 1,
    recipientId: controllerId,
    keyEpoch: 3,
    sequence,
    createdAtMs: 1n,
    nonce: new Uint8Array(24).fill(Number(sequence)),
  };
}

function inboundRelay(
  referencedBlob: Uint8Array,
  downloadedBlob: Uint8Array,
  events: string[],
  onAccept?: (sequence: bigint) => void,
  consumeStatus = 204,
): EncryptedAnywhereRelay {
  const reference = {
    blob_id: blobId,
    ciphertext_bytes: referencedBlob.length,
    ciphertext_sha256: base64Url(sha256(referencedBlob)),
  };
  installWebSocket((socket, value) => {
    const request = openEnvelope(value, dataKey, ed25519.getPublicKey(controllerSeed));
    const payload = JSON.parse(new TextDecoder().decode(request.plaintext)) as { request_id: number[] };
    socket.emit(hostEnvelope(2, {
      request_id: payload.request_id,
      status: 200,
      body_blob: reference,
    }, 9n));
  });
  globalThis.fetch = vi.fn(async (input, init) => {
    const url = input.toString();
    if (url.endsWith("/v1/relay/tickets")) return jsonResponse({ ticket: "ticket" });
    if (url.endsWith(`/v1/relay/blobs/${blobId}`) && init?.method === "GET") {
      events.push("claim");
      return jsonResponse({ ...reference, download_url: "https://objects.test/download" });
    }
    if (url === "https://objects.test/download") {
      events.push("download");
      expect(new Headers(init?.headers).has("authorization")).toBe(false);
      return new Response(downloadedBlob as unknown as BodyInit, { status: 200 });
    }
    if (url.endsWith(`/v1/relay/blobs/${blobId}`) && init?.method === "DELETE") {
      expect(new Headers(init.headers).get("Idempotency-Key")).toMatch(/^[0-9a-f]{32}$/);
      events.push("consume");
      return new Response(null, { status: consumeStatus });
    }
    throw new Error(`unexpected fetch ${url}`);
  }) as typeof fetch;
  return new EncryptedAnywhereRelay(credentials(onAccept));
}

function blobUploadFetch(): typeof fetch {
  return vi.fn(async (input, init) => {
    const url = input.toString();
    if (url.endsWith("/v1/relay/tickets")) return jsonResponse({ ticket: "ticket" });
    if (url.endsWith("/v1/relay/blobs") && init?.method === "POST") {
      return jsonResponse({ blob_id: blobId, upload_url: "https://objects.test/upload" });
    }
    if (url === "https://objects.test/upload") return new Response(null, { status: 200 });
    if (url.endsWith(`/v1/relay/blobs/${blobId}/complete`)) return new Response(null, { status: 204 });
    throw new Error(`unexpected fetch ${url}`);
  }) as typeof fetch;
}

function installWebSocket(onSend: (socket: MockWebSocket, value: Uint8Array) => void): void {
  class InstalledWebSocket extends MockWebSocket {
    override send(value: Uint8Array): void {
      onSend(this, value);
    }
  }
  globalThis.WebSocket = InstalledWebSocket as unknown as typeof WebSocket;
}

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

  send(_value: Uint8Array): void {}

  emit(value: Uint8Array): void {
    queueMicrotask(() => this.onmessage?.({ data: value.buffer as ArrayBuffer }));
  }

  close(): void {
    this.readyState = MockWebSocket.CLOSED;
    this.onclose?.();
  }
}

function jsonResponse(value: unknown): Response {
  return new Response(JSON.stringify(value), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
}

function base64Url(bytes: Uint8Array): string {
  const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
  let output = "";
  for (let index = 0; index < bytes.length; index += 3) {
    const first = bytes[index];
    const second = bytes[index + 1];
    const third = bytes[index + 2];
    output += alphabet[first >>> 2];
    output += alphabet[((first & 3) << 4) | ((second ?? 0) >>> 4)];
    if (second !== undefined) output += alphabet[((second & 15) << 2) | ((third ?? 0) >>> 6)];
    if (third !== undefined) output += alphabet[third & 63];
  }
  return output;
}
