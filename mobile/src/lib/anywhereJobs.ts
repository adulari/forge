import { base64Url, fromBase64Url } from "./anywhereApi";
import {
  bytesFromHex,
  bytesToHex,
  decodeEnvelope,
  openEnvelope,
  sealEnvelope,
} from "./transport/anywhereEnvelope";

const COMMAND_TTL_MS = 24 * 60 * 60 * 1000;
const MAX_COMMAND_BYTES = 256 * 1024;

export type RemoteJobResult =
  | { status: "success" }
  | { status: "error"; code: "invalid_command" | "permission_denied" | "host_unavailable" | "execution_failed"; retryable: boolean };

export interface CreateSessionJob {
  hostId: string;
  hostDeviceId: string;
  cwd?: string;
  worktree?: boolean;
  title?: string;
  model?: string;
  temper?: "Read-only" | "Ask" | "Auto-edit" | "Full";
}

export interface PendingRemoteJob {
  localId: string;
  hostId: string;
  hostDeviceId: string;
  createdAtMs: number;
  envelope: string;
  idempotencyKey: string;
  commandId?: string;
  expiresAtMs?: number;
  result?: RemoteJobResult;
}

export interface AnywhereJobStore {
  load(): Promise<PendingRemoteJob[]>;
  save(jobs: PendingRemoteJob[]): Promise<void>;
}

export function parseStoredRemoteJobs(encoded: string): PendingRemoteJob[] {
  const jobs = JSON.parse(encoded) as unknown;
  if (!Array.isArray(jobs) || !jobs.every(isStoredJob)) throw new Error("Stored Anywhere remote jobs are invalid");
  return jobs;
}

export interface AnywhereJobCredentials {
  serviceUrl: string;
  accountId: Uint8Array;
  deviceId: Uint8Array;
  dataKey: Uint8Array;
  dataKeyForEpoch?(epoch: number): Promise<Uint8Array>;
  keyEpoch: number;
  signingPrivateKey: Uint8Array;
  accessToken(): Promise<string>;
  reserveSequence(): Promise<bigint>;
  acceptSequences(senderDeviceId: string, epoch: number, sequences: readonly bigint[]): Promise<boolean>;
  signingPublicKey(senderDeviceId: string): Promise<Uint8Array>;
  randomBytes(length: number): Uint8Array;
  now?(): number;
}

interface EnqueueResponse {
  version: number;
  command_id: string;
  expires_at_ms: number;
  already_queued: boolean;
}

interface CommandAcknowledgement {
  command_id: string;
  result: RemoteJobResult;
}

/** Durable producer: persist first, then retry the same ciphertext and idempotency key. */
export class AnywhereJobClient {
  private mutation = Promise.resolve();

  constructor(
    private readonly credentials: AnywhereJobCredentials,
    private readonly store: AnywhereJobStore,
    private readonly request: typeof fetch = globalThis.fetch.bind(globalThis),
  ) {
    assertBytes("account id", credentials.accountId, 16);
    assertBytes("device id", credentials.deviceId, 16);
    assertBytes("Account Data Key", credentials.dataKey, 32);
    assertBytes("signing private key", credentials.signingPrivateKey, 32);
  }

  queueCreateSession(input: CreateSessionJob): Promise<PendingRemoteJob> {
    return this.exclusive(async () => {
      const hostId = bytesFromHex(input.hostId);
      assertBytes("host id", hostId, 16);
      assertBytes("host device id", bytesFromHex(input.hostDeviceId), 16);
      const sequence = await this.credentials.reserveSequence();
      const requestId = this.credentials.randomBytes(16);
      assertBytes("request id", requestId, 16);
      const body = new TextEncoder().encode(JSON.stringify(compact({
        cwd: input.cwd,
        worktree: input.worktree ?? false,
        title: input.title,
        model: input.model,
        temper: input.temper,
      })));
      // Key order matches the normative Rust BridgeRequest serializer. The host rejects alternate
      // JSON encodings so retries and cross-language implementations have one plaintext shape.
      const plaintext = new TextEncoder().encode(JSON.stringify({
        request_id: Array.from(requestId),
        route: "create_session",
        method: "POST",
        parameters: [],
        headers: [["content-type", "application/json"]],
        body: Array.from(body),
      }));
      const createdAtMs = this.now();
      const envelope = sealEnvelope({
        kind: 9,
        flags: 0,
        accountId: this.credentials.accountId,
        senderDeviceId: this.credentials.deviceId,
        recipientKind: 2,
        recipientId: hostId,
        keyEpoch: this.credentials.keyEpoch,
        sequence,
        createdAtMs: BigInt(createdAtMs),
        nonce: this.credentials.randomBytes(24),
      }, plaintext, this.credentials.dataKey, this.credentials.signingPrivateKey);
      if (envelope.length > MAX_COMMAND_BYTES) throw new Error("Encrypted remote job exceeds 256 KiB");
      const job: PendingRemoteJob = {
        localId: bytesToHex(requestId),
        hostId: input.hostId,
        hostDeviceId: input.hostDeviceId,
        createdAtMs,
        envelope: base64Url(envelope),
        idempotencyKey: bytesToHex(this.credentials.randomBytes(16)),
      };
      const jobs = await this.store.load();
      await this.store.save([...jobs, job]);
      return this.submit(job);
    });
  }

  resume(): Promise<PendingRemoteJob[]> {
    return this.exclusive(async () => {
      const jobs = await this.store.load();
      const updated: PendingRemoteJob[] = [];
      for (const job of jobs) {
        if (job.result || (job.expiresAtMs != null && job.expiresAtMs <= this.now())) {
          updated.push(job);
          continue;
        }
        try { updated.push(await this.submit(job)); }
        catch {
          // Submission may have been accepted and persisted before a later acknowledgement poll
          // failed. Reload so an offline poll cannot roll that durable progress back.
          const latest = (await this.store.load()).find((candidate) => candidate.localId === job.localId);
          updated.push(latest ?? job);
        }
      }
      await this.store.save(updated);
      return updated;
    });
  }

  private async submit(job: PendingRemoteJob): Promise<PendingRemoteJob> {
    let next = job;
    const token = await this.credentials.accessToken();
    if (!next.commandId) {
      const response = await this.request(`${stripSlash(this.credentials.serviceUrl)}/v1/hosts/${next.hostId}/commands`, {
        method: "POST",
        headers: {
          accept: "application/json",
          authorization: `Bearer ${token}`,
          "content-type": "application/octet-stream",
          "Idempotency-Key": next.idempotencyKey,
        },
        body: fromBase64Url(next.envelope) as unknown as BodyInit,
      });
      if (!response.ok) throw new Error(`Queue remote job failed (${response.status})`);
      const queued = await response.json() as EnqueueResponse;
      validateEnqueue(queued, this.now());
      next = { ...next, commandId: queued.command_id, expiresAtMs: queued.expires_at_ms };
      await this.replace(next);
    }
    if ((next.expiresAtMs ?? 0) <= this.now()) return next;
    const ack = await this.request(`${stripSlash(this.credentials.serviceUrl)}/v1/hosts/${next.hostId}/commands/${next.commandId}/ack`, {
      headers: { accept: "application/octet-stream", authorization: `Bearer ${token}` },
    });
    if (ack.status === 404) return next;
    if (!ack.ok) throw new Error(`Poll remote job failed (${ack.status})`);
    const encoded = new Uint8Array(await ack.arrayBuffer());
    const decoded = decodeEnvelope(encoded);
    const sender = bytesToHex(decoded.metadata.senderDeviceId);
    if (decoded.metadata.kind !== 10
      || decoded.metadata.recipientKind !== 1
      || sender !== next.hostDeviceId
      || !equal(decoded.metadata.accountId, this.credentials.accountId)
      || !equal(decoded.metadata.recipientId, this.credentials.deviceId)) {
      throw new Error("Remote job acknowledgement has invalid routing metadata");
    }
    const signingKey = await this.credentials.signingPublicKey(sender);
    const dataKey = decoded.metadata.keyEpoch === this.credentials.keyEpoch
      ? this.credentials.dataKey
      : await this.credentials.dataKeyForEpoch?.(decoded.metadata.keyEpoch);
    if (!dataKey) throw new Error("Remote job acknowledgement uses an unavailable key epoch");
    const opened = openEnvelope(encoded, dataKey, signingKey);
    if (!await this.credentials.acceptSequences(sender, decoded.metadata.keyEpoch, [decoded.metadata.sequence])) {
      throw new Error("Remote job acknowledgement replay rejected");
    }
    const acknowledgement = JSON.parse(new TextDecoder().decode(opened.plaintext)) as CommandAcknowledgement;
    validateAcknowledgement(acknowledgement, next.commandId as string);
    next = { ...next, result: acknowledgement.result };
    await this.replace(next);
    return next;
  }

  private async replace(job: PendingRemoteJob): Promise<void> {
    const jobs = await this.store.load();
    const index = jobs.findIndex((candidate) => candidate.localId === job.localId);
    if (index < 0) throw new Error("Remote job disappeared from its protected queue");
    const next = [...jobs];
    next[index] = job;
    await this.store.save(next);
  }

  private exclusive<T>(operation: () => Promise<T>): Promise<T> {
    const result = this.mutation.catch(() => undefined).then(operation);
    this.mutation = result.then(() => undefined, () => undefined);
    return result;
  }

  private now(): number { return this.credentials.now?.() ?? Date.now(); }
}

export type GenericPushEvent = "attention_required" | "job_completed" | "job_failed" | "workspace_ready";

/** Fixed-category, idempotent, best-effort notification trigger. Never throws. */
export async function requestGenericPush(
  serviceUrl: string,
  accessToken: string,
  event: GenericPushEvent,
  idempotencyKey: string,
  targetDeviceId?: string,
  request: typeof fetch = globalThis.fetch.bind(globalThis),
): Promise<void> {
  try {
    await request(`${stripSlash(serviceUrl)}/v1/push/notifications`, {
      method: "POST",
      headers: {
        authorization: `Bearer ${accessToken}`,
        "content-type": "application/json",
        "Idempotency-Key": idempotencyKey,
      },
      body: JSON.stringify(compact({ event, target_device_id: targetDeviceId })),
    });
  } catch { /* push is only a refresh hint; core work already succeeded */ }
}

function validateEnqueue(response: EnqueueResponse, createdAtMs: number): void {
  if (response.version !== 1 || !/^[0-9a-f]{32}$/.test(response.command_id)) throw new Error("Invalid remote job enqueue response");
  if (!Number.isSafeInteger(response.expires_at_ms)
    || Math.abs(response.expires_at_ms - (createdAtMs + COMMAND_TTL_MS)) > 5 * 60 * 1000) {
    throw new Error("Invalid remote job expiry");
  }
}

function validateAcknowledgement(value: CommandAcknowledgement, commandId: string): void {
  if (value.command_id !== commandId || value.result == null || !["success", "error"].includes(value.result.status)) {
    throw new Error("Invalid remote job acknowledgement");
  }
  if (value.result.status === "error"
    && (!new Set(["invalid_command", "permission_denied", "host_unavailable", "execution_failed"]).has(value.result.code)
      || typeof value.result.retryable !== "boolean")) {
    throw new Error("Invalid remote job error category");
  }
}

function isStoredJob(value: unknown): value is PendingRemoteJob {
  if (value == null || typeof value !== "object") return false;
  const job = value as Partial<PendingRemoteJob>;
  return typeof job.localId === "string"
    && typeof job.hostId === "string"
    && typeof job.hostDeviceId === "string"
    && typeof job.createdAtMs === "number"
    && typeof job.envelope === "string"
    && typeof job.idempotencyKey === "string"
    && (job.commandId === undefined || typeof job.commandId === "string")
    && (job.expiresAtMs === undefined || typeof job.expiresAtMs === "number")
    && (job.result === undefined || job.result.status === "success" || job.result.status === "error");
}

function compact<T extends Record<string, unknown>>(value: T): Partial<T> {
  return Object.fromEntries(Object.entries(value).filter(([, item]) => item !== undefined)) as Partial<T>;
}

function stripSlash(value: string): string { return value.replace(/\/$/, ""); }

function equal(left: Uint8Array, right: Uint8Array): boolean {
  return left.length === right.length && left.every((byte, index) => byte === right[index]);
}

function assertBytes(label: string, value: Uint8Array, length: number): void {
  if (value.length !== length) throw new Error(`${label} must contain ${length} bytes`);
}
