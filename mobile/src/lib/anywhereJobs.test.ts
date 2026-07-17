import { ed25519 } from "@noble/curves/ed25519.js";
import { describe, expect, it, vi } from "vitest";

import {
  AnywhereJobClient,
  requestGenericPush,
  type AnywhereJobCredentials,
  type AnywhereJobStore,
  type PendingRemoteJob,
} from "./anywhereJobs";
import { base64Url } from "./anywhereApi";
import { bytesToHex, sealEnvelope } from "./transport/anywhereEnvelope";

const account = new Uint8Array(16).fill(0x11);
const controller = new Uint8Array(16).fill(0x22);
const host = new Uint8Array(16).fill(0x33);
const hostDevice = new Uint8Array(16).fill(0x44);
const dataKey = new Uint8Array(32).fill(0x55);
const controllerSigning = new Uint8Array(32).fill(0x66);
const hostSigning = new Uint8Array(32).fill(0x77);
const commandId = "88".repeat(16);

function memoryStore(): AnywhereJobStore & { jobs: PendingRemoteJob[] } {
  return {
    jobs: [],
    async load() { return structuredClone(this.jobs); },
    async save(jobs) { this.jobs = structuredClone(jobs); },
  };
}

function credentials(): AnywhereJobCredentials {
  let sequence = 1n;
  return {
    serviceUrl: "https://app.example",
    accountId: account,
    deviceId: controller,
    dataKey,
    dataKeyForEpoch: async () => dataKey,
    keyEpoch: 1,
    signingPrivateKey: controllerSigning,
    accessToken: async () => "access-secret",
    reserveSequence: async () => sequence++,
    acceptSequences: async () => true,
    signingPublicKey: async () => ed25519.getPublicKey(hostSigning),
    randomBytes: (length) => new Uint8Array(length).fill(length),
    now: () => 1_750_000_000_000,
  };
}

describe("Anywhere durable remote jobs", () => {
  it("persists exact ciphertext before a failed send and reuses it on resume", async () => {
    const store = memoryStore();
    const bodies: string[] = [];
    let fail = true;
    const request = vi.fn(async (url: RequestInfo | URL, init?: RequestInit) => {
      if (String(url).endsWith("/ack")) return new Response(null, { status: 404 });
      const bytes = init?.body as unknown as Uint8Array;
      bodies.push(base64Url(bytes));
      if (fail) { fail = false; throw new Error("offline"); }
      return new Response(JSON.stringify({
        version: 1,
        command_id: commandId,
        expires_at_ms: 1_750_086_400_000,
        already_queued: false,
      }), { status: 200, headers: { "content-type": "application/json" } });
    }) as typeof fetch;
    const client = new AnywhereJobClient(credentials(), store, request);

    await expect(client.queueCreateSession({
      hostId: bytesToHex(host),
      hostDeviceId: bytesToHex(hostDevice),
      cwd: "/private/repository",
      title: "secret title",
    })).rejects.toThrow("offline");
    expect(store.jobs).toHaveLength(1);
    expect(JSON.stringify(store.jobs)).not.toContain("private/repository");
    expect(JSON.stringify(store.jobs)).not.toContain("secret title");

    await client.resume();
    expect(bodies[1]).toBe(bodies[0]);
    expect(store.jobs[0].commandId).toBe(commandId);
  });

  it("authenticates an acknowledgement and exposes only its categorical result", async () => {
    const store = memoryStore();
    const acknowledgement = sealEnvelope({
      kind: 10,
      flags: 0,
      accountId: account,
      senderDeviceId: hostDevice,
      recipientKind: 1,
      recipientId: controller,
      keyEpoch: 1,
      sequence: 9n,
      createdAtMs: 1_750_000_000_100n,
      nonce: new Uint8Array(24).fill(9),
    }, new TextEncoder().encode(JSON.stringify({ command_id: commandId, result: { status: "success" } })), dataKey, hostSigning);
    let calls = 0;
    const request = vi.fn(async () => {
      calls += 1;
      if (calls === 1) return new Response(JSON.stringify({
        version: 1, command_id: commandId, expires_at_ms: 1_750_086_400_000, already_queued: false,
      }), { status: 200, headers: { "content-type": "application/json" } });
      return new Response(acknowledgement as unknown as BodyInit, { status: 200, headers: { "content-type": "application/octet-stream" } });
    }) as typeof fetch;

    const result = await new AnywhereJobClient(credentials(), store, request).queueCreateSession({
      hostId: bytesToHex(host),
      hostDeviceId: bytesToHex(hostDevice),
    });
    expect(result.result).toEqual({ status: "success" });
    expect(Object.keys(result.result as object)).toEqual(["status"]);
  });

  it("requests only a fixed generic push category and never propagates failure", async () => {
    const request = vi.fn(async (_url: RequestInfo | URL, init?: RequestInit) => {
      expect(JSON.parse(init?.body as string)).toEqual({
        event: "workspace_ready",
        target_device_id: "22".repeat(16),
      });
      throw new Error("push unavailable");
    }) as typeof fetch;
    await expect(requestGenericPush(
      "https://app.example",
      "access-secret",
      "workspace_ready",
      "capsule-stable-ready",
      "22".repeat(16),
      request,
    )).resolves.toBeUndefined();
  });
});
