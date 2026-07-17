import type {
  AnywhereBridgeRequest,
  AnywhereBridgeResponse,
  AnywhereRelay,
} from "./AnywhereTransport";
import { sha256 } from "@noble/hashes/sha2.js";
import type { RemoteSocket } from "./RemoteTransport";
import {
  bytesFromHex,
  bytesToHex,
  decodeEnvelope,
  openEnvelope,
  sealEnvelope,
  type EnvelopeKind,
} from "./anywhereEnvelope";

const MAX_INLINE_BYTES = 256 * 1024;
const MAX_BLOB_CIPHERTEXT_BYTES = 32 * 1024 * 1024;
const REQUEST_TIMEOUT_MS = 30_000;

export interface RelayBlobReference {
  blob_id: string;
  ciphertext_bytes: number;
  ciphertext_sha256: string;
}

export interface AnywhereRelayCredentials {
  serviceUrl: string;
  accountId: Uint8Array;
  deviceId: Uint8Array;
  dataKey: Uint8Array;
  keyEpoch: number;
  signingPrivateKey: Uint8Array;
  accessToken(): Promise<string>;
  /** Persistently reserves a never-reused sequence before returning it. */
  reserveSequence(): Promise<bigint>;
  /** Atomically accepts ordered envelope sequences and persists the accepted maximum. */
  acceptSequences(
    senderDeviceId: string,
    keyEpoch: number,
    sequences: readonly bigint[],
  ): Promise<boolean>;
  signingPublicKey(senderDeviceId: string): Promise<Uint8Array>;
  randomBytes(length: number): Uint8Array;
}

interface PendingRequest {
  resolve(response: AnywhereBridgeResponse): void;
  reject(error: Error): void;
  timeout: ReturnType<typeof setTimeout>;
}

interface BridgeResponsePayload {
  request_id: number[];
  status: number;
  headers?: [string, string][];
  body?: number[];
  body_blob?: RelayBlobReference;
}

interface WebSocketPayload {
  stream_id: number[];
  direction: "controller_to_host" | "host_to_controller";
  kind: "data" | "close";
  bytes?: number[];
  bytes_blob?: RelayBlobReference;
}

interface BlobReservation {
  blob_id?: string;
  upload_url?: string;
  required_headers?: Record<string, string>;
  already_complete?: boolean;
}

interface BlobClaim {
  blob_id?: string;
  ciphertext_bytes?: number;
  ciphertext_sha256?: string;
  download_url?: string;
  required_headers?: Record<string, string>;
}

interface PreparedDelivery {
  consume?: () => Promise<void>;
  precedingSequences?: readonly bigint[];
  deliver(): void;
}

/** Ticketed, end-to-end encrypted implementation backing AnywhereTransport. */
export class EncryptedAnywhereRelay implements AnywhereRelay {
  private connection: WebSocket | null = null;
  private connecting: Promise<WebSocket> | null = null;
  private incoming = Promise.resolve();
  private readonly pending = new Map<string, PendingRequest>();
  private readonly streams = new Map<string, AnywhereRemoteSocket>();

  constructor(private readonly credentials: AnywhereRelayCredentials) {
    assertLength("account id", credentials.accountId, 16);
    assertLength("device id", credentials.deviceId, 16);
    assertLength("Account Data Key", credentials.dataKey, 32);
    assertLength("signing private key", credentials.signingPrivateKey, 32);
  }

  async request(request: AnywhereBridgeRequest): Promise<AnywhereBridgeResponse> {
    const requestId = this.credentials.randomBytes(16);
    assertLength("request id", requestId, 16);
    const payload: Record<string, unknown> = {
      request_id: Array.from(requestId),
      route: request.route,
      method: request.method,
      parameters: request.parameters,
      headers: request.headers,
    };
    if (request.body.length > MAX_INLINE_BYTES) {
      payload.body_blob = await this.uploadBlob(request.hostId, request.body);
    } else {
      payload.body = Array.from(request.body);
    }
    return this.sendBridgeRequest(request.hostId, payload);
  }

  openSessionSocket(request: {
    hostId: string;
    sessionId: string;
    revision: number;
  }): RemoteSocket {
    const streamId = this.credentials.randomBytes(16);
    assertLength("stream id", streamId, 16);
    const socket = new AnywhereRemoteSocket(streamId, request.hostId, this);
    this.streams.set(bytesToHex(streamId), socket);
    void this.sendBridgeRequest(request.hostId, {
      request_id: Array.from(streamId),
      route: "web_socket",
      method: "GET",
      parameters: [request.sessionId, request.revision.toString()],
      headers: [],
      body: [],
    }).then(
      (response) => {
        if (response.status >= 200 && response.status < 300) socket.markOpen();
        else socket.fail(new Error(`Anywhere stream open failed with HTTP ${response.status}`));
      },
      (error) => socket.fail(asError(error)),
    );
    return socket;
  }

  async sendStreamFrame(hostId: string, payload: WebSocketPayload): Promise<void> {
    let wirePayload = payload;
    const frameBytes = new Uint8Array(payload.bytes ?? []);
    if (payload.kind === "data" && frameBytes.length > MAX_INLINE_BYTES) {
      wirePayload = {
        stream_id: payload.stream_id,
        direction: payload.direction,
        kind: payload.kind,
        bytes_blob: await this.uploadBlob(hostId, frameBytes),
      };
    }
    const bytes = new TextEncoder().encode(JSON.stringify(wirePayload));
    const envelope = await this.seal(3, hostId, bytes);
    (await this.ensureConnection()).send(envelope);
  }

  removeStream(streamId: Uint8Array): void {
    this.streams.delete(bytesToHex(streamId));
  }

  private async sendBridgeRequest(
    hostId: string,
    payload: Record<string, unknown>,
  ): Promise<AnywhereBridgeResponse> {
    const requestId = new Uint8Array(payload.request_id as number[]);
    const key = bytesToHex(requestId);
    const plaintext = new TextEncoder().encode(JSON.stringify(payload));
    const envelope = await this.seal(1, hostId, plaintext);
    const connection = await this.ensureConnection();
    return new Promise<AnywhereBridgeResponse>((resolve, reject) => {
      const timeout = setTimeout(() => {
        this.pending.delete(key);
        reject(new Error("Forge Anywhere relay request timed out"));
      }, REQUEST_TIMEOUT_MS);
      this.pending.set(key, { resolve, reject, timeout });
      try {
        connection.send(envelope);
      } catch (error) {
        clearTimeout(timeout);
        this.pending.delete(key);
        reject(asError(error));
      }
    });
  }

  private async seal(kind: EnvelopeKind, hostId: string, plaintext: Uint8Array): Promise<Uint8Array> {
    const recipientId = bytesFromHex(hostId);
    assertLength("host id", recipientId, 16);
    const nonce = this.credentials.randomBytes(24);
    assertLength("nonce", nonce, 24);
    // reserveSequence persists first; a crash may create a gap but can never create a replay.
    const sequence = await this.credentials.reserveSequence();
    return sealEnvelope(
      {
        kind,
        flags: 0,
        accountId: this.credentials.accountId,
        senderDeviceId: this.credentials.deviceId,
        recipientKind: 2,
        recipientId,
        keyEpoch: this.credentials.keyEpoch,
        sequence,
        createdAtMs: BigInt(Date.now()),
        nonce,
      },
      plaintext,
      this.credentials.dataKey,
      this.credentials.signingPrivateKey,
    );
  }

  private async uploadBlob(hostId: string, plaintext: Uint8Array): Promise<RelayBlobReference> {
    const envelope = await this.seal(8, hostId, plaintext);
    if (envelope.length > MAX_BLOB_CIPHERTEXT_BYTES) {
      throw new Error("Anywhere relay blob exceeds the 32 MiB ciphertext limit");
    }
    const reference = {
      ciphertext_bytes: envelope.length,
      ciphertext_sha256: base64Url(sha256(envelope)),
    };
    const reservationKey = this.idempotencyKey();
    const reservationResponse = await this.serviceFetchRetryOnce("/v1/relay/blobs", {
      method: "POST",
      headers: {
        "content-type": "application/json",
        "Idempotency-Key": reservationKey,
      },
      body: JSON.stringify({
        recipient_kind: "host",
        recipient_id: hostId,
        ...reference,
      }),
    });
    await requireOk(reservationResponse, "reserve relay blob");
    const reservation = await jsonObject<BlobReservation>(reservationResponse, "relay blob reservation");
    const blobId = requireBlobId(reservation.blob_id);
    if (!reservation.already_complete) {
      if (typeof reservation.upload_url !== "string" || reservation.upload_url.length === 0) {
        throw new Error("Forge Anywhere relay blob reservation omitted its upload URL");
      }
      const upload = await fetch(reservation.upload_url, {
        method: "PUT",
        headers: serviceHeaders(reservation.required_headers),
        body: envelope as unknown as BodyInit,
      });
      await requireOk(upload, "upload relay blob");
      const completionKey = this.idempotencyKey();
      const complete = await this.serviceFetchRetryOnce(`/v1/relay/blobs/${blobId}/complete`, {
        method: "POST",
        headers: { "Idempotency-Key": completionKey },
      });
      await requireOk(complete, "complete relay blob");
    }
    return { blob_id: blobId, ...reference };
  }

  private async prepareBlob(
    reference: RelayBlobReference,
    expectedSenderId: string,
  ): Promise<{ plaintext: Uint8Array; sequence: bigint; consume: () => Promise<void> }> {
    assertBlobReference(reference);
    const claimResponse = await this.serviceFetch(`/v1/relay/blobs/${reference.blob_id}`, {
      method: "GET",
    });
    await requireOk(claimResponse, "claim relay blob");
    const claim = await jsonObject<BlobClaim>(claimResponse, "relay blob claim");
    if (claim.blob_id !== undefined && requireBlobId(claim.blob_id) !== reference.blob_id) {
      throw new Error("Anywhere relay blob claim ID does not match its reference");
    }
    if (claim.ciphertext_bytes !== reference.ciphertext_bytes) {
      throw new Error("Anywhere relay blob claim length does not match its reference");
    }
    if (claim.ciphertext_sha256 !== reference.ciphertext_sha256) {
      throw new Error("Anywhere relay blob claim hash does not match its reference");
    }
    if (typeof claim.download_url !== "string" || claim.download_url.length === 0) {
      throw new Error("Forge Anywhere relay blob claim omitted its download URL");
    }
    const download = await fetch(claim.download_url, {
      method: "GET",
      headers: serviceHeaders(claim.required_headers),
    });
    await requireOk(download, "download relay blob");
    const envelope = await responseBytes(download, reference.ciphertext_bytes);
    if (envelope.length !== reference.ciphertext_bytes) {
      throw new Error("Anywhere relay blob ciphertext length mismatch");
    }
    if (base64Url(sha256(envelope)) !== reference.ciphertext_sha256) {
      throw new Error("Anywhere relay blob ciphertext SHA-256 mismatch");
    }

    const decoded = decodeEnvelope(envelope);
    if (decoded.metadata.kind !== 8) throw new Error("Anywhere relay blob has the wrong envelope kind");
    if (!equal(decoded.metadata.accountId, this.credentials.accountId)) {
      throw new Error("Anywhere relay blob account does not match this device");
    }
    if (decoded.metadata.recipientKind !== 1 || !equal(decoded.metadata.recipientId, this.credentials.deviceId)) {
      throw new Error("Anywhere relay blob is not addressed to this device");
    }
    if (decoded.metadata.keyEpoch !== this.credentials.keyEpoch) {
      throw new Error("Anywhere relay blob uses an unavailable key epoch");
    }
    const senderId = bytesToHex(decoded.metadata.senderDeviceId);
    if (senderId !== expectedSenderId) {
      throw new Error("Anywhere relay blob sender does not match its reference envelope");
    }
    const signingKey = await this.credentials.signingPublicKey(senderId);
    const opened = openEnvelope(envelope, this.credentials.dataKey, signingKey);
    return {
      plaintext: opened.plaintext,
      sequence: opened.metadata.sequence,
      consume: async () => {
        const response = await this.serviceFetch(`/v1/relay/blobs/${reference.blob_id}`, {
          method: "DELETE",
          headers: { "Idempotency-Key": this.idempotencyKey() },
        });
        await requireOk(response, "consume relay blob");
      },
    };
  }

  private async serviceFetch(path: string, init: RequestInit): Promise<Response> {
    const headers = new Headers(init.headers);
    headers.set("authorization", `Bearer ${await this.credentials.accessToken()}`);
    return fetch(`${trimSlash(this.credentials.serviceUrl)}${path}`, { ...init, headers });
  }

  private async serviceFetchRetryOnce(path: string, init: RequestInit): Promise<Response> {
    try {
      const response = await this.serviceFetch(path, init);
      if (response.status < 500) return response;
    } catch {
      // Retry below with the same idempotency key and request body.
    }
    return this.serviceFetch(path, init);
  }

  private idempotencyKey(): string {
    const bytes = this.credentials.randomBytes(16);
    assertLength("idempotency key", bytes, 16);
    return bytesToHex(bytes);
  }

  private async ensureConnection(): Promise<WebSocket> {
    if (this.connection?.readyState === WebSocket.OPEN) return this.connection;
    if (this.connecting) return this.connecting;
    this.connecting = this.connect().finally(() => {
      this.connecting = null;
    });
    return this.connecting;
  }

  private async connect(): Promise<WebSocket> {
    const accessToken = await this.credentials.accessToken();
    const response = await fetch(`${trimSlash(this.credentials.serviceUrl)}/v1/relay/tickets`, {
      method: "POST",
      headers: {
        authorization: `Bearer ${accessToken}`,
        "content-type": "application/json",
      },
      body: JSON.stringify({ device_id: bytesToHex(this.credentials.deviceId) }),
    });
    if (!response.ok) throw new Error(`Forge Anywhere relay ticket failed with HTTP ${response.status}`);
    const ticket = (await response.json()) as { ticket?: string };
    if (!ticket.ticket) throw new Error("Forge Anywhere service returned no relay ticket");
    const url = new URL(`${trimSlash(this.credentials.serviceUrl)}/v1/relay`);
    url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
    url.searchParams.set("ticket", ticket.ticket);
    const socket = new WebSocket(url.toString());
    socket.binaryType = "arraybuffer";
    await new Promise<void>((resolve, reject) => {
      socket.onopen = () => resolve();
      socket.onerror = () => reject(new Error("Forge Anywhere relay WebSocket failed to connect"));
    });
    socket.onmessage = (event) => {
      this.incoming = this.incoming.then(() => this.handleEnvelope(event.data));
    };
    socket.onclose = () => this.connectionClosed();
    socket.onerror = () => this.connectionClosed();
    this.connection = socket;
    return socket;
  }

  private async handleEnvelope(data: unknown): Promise<void> {
    try {
      const bytes = await socketBytes(data);
      const decoded = decodeEnvelope(bytes);
      if (!equal(decoded.metadata.accountId, this.credentials.accountId)) {
        throw new Error("Anywhere response account does not match this device");
      }
      if (decoded.metadata.recipientKind !== 1 || !equal(decoded.metadata.recipientId, this.credentials.deviceId)) {
        throw new Error("Anywhere response is not addressed to this device");
      }
      if (decoded.metadata.keyEpoch !== this.credentials.keyEpoch) {
        throw new Error("Anywhere response uses an unavailable key epoch");
      }
      const senderId = bytesToHex(decoded.metadata.senderDeviceId);
      const signingKey = await this.credentials.signingPublicKey(senderId);
      const opened = openEnvelope(bytes, this.credentials.dataKey, signingKey);
      let prepared: PreparedDelivery;
      if (opened.metadata.kind === 2) prepared = await this.prepareBridgeResponse(opened.plaintext, senderId);
      else if (opened.metadata.kind === 3) prepared = await this.prepareStreamFrame(opened.plaintext, senderId);
      else throw new Error("unexpected Anywhere envelope kind on controller relay");
      if (prepared.precedingSequences?.some((sequence) => sequence >= opened.metadata.sequence)) {
        throw new Error("Anywhere relay blob sequence does not precede its reference");
      }
      const sequences = [...(prepared.precedingSequences ?? []), opened.metadata.sequence];
      if (!(await this.credentials.acceptSequences(senderId, opened.metadata.keyEpoch, sequences))) {
        throw new Error("replayed or out-of-order Anywhere response");
      }
      await prepared.consume?.().catch(() => undefined);
      prepared.deliver();
    } catch (error) {
      const connection = this.connection;
      this.connectionClosed(asError(error));
      connection?.close(1002, "invalid encrypted relay frame");
    }
  }

  private async prepareBridgeResponse(plaintext: Uint8Array, senderId: string): Promise<PreparedDelivery> {
    const payload = JSON.parse(new TextDecoder().decode(plaintext)) as BridgeResponsePayload;
    const requestId = new Uint8Array(payload.request_id);
    assertLength("bridge response request id", requestId, 16);
    const pending = this.pending.get(bytesToHex(requestId));
    assertOptionalByteArray("bridge response body", payload.body);
    if (payload.body_blob !== undefined) assertBlobReference(payload.body_blob);
    if (payload.body_blob !== undefined && (payload.body?.length ?? 0) !== 0) {
      throw new Error("Anywhere bridge response contains both inline and blob bodies");
    }
    if (payload.body_blob === undefined && (payload.body?.length ?? 0) > MAX_INLINE_BYTES) {
      throw new Error("Anywhere bridge response inline body exceeds 256 KiB");
    }
    if (!pending) return { deliver: () => {} };
    const preparedBlob = payload.body_blob === undefined
      ? undefined
      : await this.prepareBlob(payload.body_blob, senderId);
    const response: AnywhereBridgeResponse = {
      status: payload.status,
      headers: payload.headers,
      body: preparedBlob?.plaintext ?? new Uint8Array(payload.body ?? []),
    };
    return {
      consume: preparedBlob?.consume,
      precedingSequences: preparedBlob === undefined ? undefined : [preparedBlob.sequence],
      deliver: () => {
        clearTimeout(pending.timeout);
        this.pending.delete(bytesToHex(requestId));
        pending.resolve(response);
      },
    };
  }

  private async prepareStreamFrame(plaintext: Uint8Array, senderId: string): Promise<PreparedDelivery> {
    const payload = JSON.parse(new TextDecoder().decode(plaintext)) as WebSocketPayload;
    if (payload.direction !== "host_to_controller") throw new Error("invalid Anywhere stream direction");
    const streamId = new Uint8Array(payload.stream_id);
    assertLength("stream id", streamId, 16);
    const stream = this.streams.get(bytesToHex(streamId));
    assertOptionalByteArray("Anywhere stream bytes", payload.bytes);
    if (payload.bytes_blob !== undefined) assertBlobReference(payload.bytes_blob);
    if (payload.bytes_blob !== undefined && (payload.bytes?.length ?? 0) !== 0) {
      throw new Error("Anywhere stream frame contains both inline and blob bytes");
    }
    if (payload.bytes_blob === undefined && (payload.bytes?.length ?? 0) > MAX_INLINE_BYTES) {
      throw new Error("Anywhere stream frame inline bytes exceed 256 KiB");
    }
    if (payload.kind === "close" && payload.bytes_blob !== undefined) {
      throw new Error("Anywhere close frame cannot contain blob bytes");
    }
    if (!stream) return { deliver: () => {} };
    const preparedBlob = payload.bytes_blob === undefined
      ? undefined
      : await this.prepareBlob(payload.bytes_blob, senderId);
    const frameBytes = preparedBlob?.plaintext ?? new Uint8Array(payload.bytes ?? []);
    return {
      consume: preparedBlob?.consume,
      precedingSequences: preparedBlob === undefined ? undefined : [preparedBlob.sequence],
      deliver: () => {
        if (payload.kind === "close") stream.remoteClose();
        else stream.receive(frameBytes);
      },
    };
  }

  private connectionClosed(error = new Error("Forge Anywhere relay disconnected")): void {
    this.connection = null;
    for (const pending of this.pending.values()) {
      clearTimeout(pending.timeout);
      pending.reject(error);
    }
    this.pending.clear();
    for (const stream of this.streams.values()) stream.fail(error);
    this.streams.clear();
  }
}

class AnywhereRemoteSocket implements RemoteSocket {
  private state: number = WebSocket.CONNECTING;
  onopen: ((event: Event) => void) | null = null;
  onmessage: ((event: MessageEvent) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;
  onclose: ((event: CloseEvent) => void) | null = null;

  constructor(
    private readonly streamId: Uint8Array,
    private readonly hostId: string,
    private readonly relay: EncryptedAnywhereRelay,
  ) {}

  get readyState(): number {
    return this.state;
  }

  markOpen(): void {
    if (this.state !== WebSocket.CONNECTING) return;
    this.state = WebSocket.OPEN;
    this.onopen?.({ type: "open" } as Event);
  }

  receive(bytes: Uint8Array): void {
    if (this.state !== WebSocket.OPEN) return;
    this.onmessage?.({ data: new TextDecoder().decode(bytes), type: "message" } as MessageEvent);
  }

  send(data: string | ArrayBufferLike | Blob | ArrayBufferView): void {
    if (this.state !== WebSocket.OPEN) throw new Error("Forge Anywhere session stream is not open");
    void dataBytes(data).then((bytes) =>
      this.relay.sendStreamFrame(this.hostId, {
        stream_id: Array.from(this.streamId),
        direction: "controller_to_host",
        kind: "data",
        bytes: Array.from(bytes),
      }),
    ).catch((error) => this.fail(asError(error)));
  }

  close(): void {
    if (this.state >= WebSocket.CLOSING) return;
    this.state = WebSocket.CLOSING;
    void this.relay.sendStreamFrame(this.hostId, {
      stream_id: Array.from(this.streamId),
      direction: "controller_to_host",
      kind: "close",
      bytes: [],
    }).finally(() => this.remoteClose());
  }

  remoteClose(): void {
    if (this.state === WebSocket.CLOSED) return;
    this.state = WebSocket.CLOSED;
    this.relay.removeStream(this.streamId);
    this.onclose?.({ code: 1000, reason: "", wasClean: true, type: "close" } as CloseEvent);
  }

  fail(_error: Error): void {
    if (this.state === WebSocket.CLOSED) return;
    this.onerror?.({ type: "error" } as Event);
    this.remoteClose();
  }
}

async function socketBytes(data: unknown): Promise<Uint8Array> {
  if (data instanceof ArrayBuffer) return new Uint8Array(data);
  if (ArrayBuffer.isView(data)) return new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
  if (data instanceof Blob) return new Uint8Array(await data.arrayBuffer());
  throw new Error("Anywhere relay requires binary WebSocket frames");
}

async function dataBytes(data: string | ArrayBufferLike | Blob | ArrayBufferView): Promise<Uint8Array> {
  if (typeof data === "string") return new TextEncoder().encode(data);
  if (data instanceof Blob) return new Uint8Array(await data.arrayBuffer());
  if (ArrayBuffer.isView(data)) return new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
  return new Uint8Array(data);
}

async function requireOk(response: Response, operation: string): Promise<void> {
  if (!response.ok) {
    throw new Error(`Forge Anywhere could not ${operation} (HTTP ${response.status})`);
  }
}

async function jsonObject<T>(response: Response, label: string): Promise<T> {
  const value: unknown = await response.json();
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`Forge Anywhere returned an invalid ${label}`);
  }
  return value as T;
}

function serviceHeaders(values: Record<string, string> | undefined): Headers {
  const headers = new Headers();
  for (const [name, value] of Object.entries(values ?? {})) headers.set(name, value);
  return headers;
}

function assertBlobReference(reference: RelayBlobReference): void {
  requireBlobId(reference?.blob_id);
  if (
    !Number.isSafeInteger(reference?.ciphertext_bytes)
    || reference.ciphertext_bytes <= 0
    || reference.ciphertext_bytes > MAX_BLOB_CIPHERTEXT_BYTES
  ) {
    throw new Error("Anywhere relay blob ciphertext length is invalid");
  }
  if (typeof reference?.ciphertext_sha256 !== "string" || !/^[A-Za-z0-9_-]{43}$/.test(reference.ciphertext_sha256)) {
    throw new Error("Anywhere relay blob ciphertext SHA-256 is invalid");
  }
}

function assertOptionalByteArray(label: string, value: number[] | undefined): void {
  if (
    value !== undefined
    && (!Array.isArray(value) || value.some((byte) => !Number.isInteger(byte) || byte < 0 || byte > 255))
  ) {
    throw new Error(`${label} must contain bytes`);
  }
}

function requireBlobId(value: unknown): string {
  if (typeof value !== "string" || !/^[0-9a-f]{32}$/i.test(value)) {
    throw new Error("Anywhere relay blob ID must be a 32-hex string");
  }
  return value.toLowerCase();
}

function base64Url(bytes: Uint8Array): string {
  const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
  let output = "";
  for (let index = 0; index < bytes.length; index += 3) {
    const first = bytes[index];
    const second = bytes[index + 1];
    const third = bytes[index + 2];
    output += alphabet[first >>> 2];
    output += alphabet[((first & 0x03) << 4) | ((second ?? 0) >>> 4)];
    if (second !== undefined) output += alphabet[((second & 0x0f) << 2) | ((third ?? 0) >>> 6)];
    if (third !== undefined) output += alphabet[third & 0x3f];
  }
  return output;
}

async function responseBytes(response: Response, maximum: number): Promise<Uint8Array> {
  const contentLength = response.headers.get("content-length");
  if (contentLength !== null) {
    const declared = Number(contentLength);
    if (!Number.isSafeInteger(declared) || declared < 0 || declared > maximum) {
      throw new Error("Anywhere relay blob download exceeds its declared length");
    }
  }
  if (!response.body) {
    const bytes = new Uint8Array(await response.arrayBuffer());
    if (bytes.length > maximum) throw new Error("Anywhere relay blob download exceeds its declared length");
    return bytes;
  }
  const reader = response.body.getReader();
  const chunks: Uint8Array[] = [];
  let length = 0;
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    length += value.length;
    if (length > maximum) {
      await reader.cancel();
      throw new Error("Anywhere relay blob download exceeds its declared length");
    }
    chunks.push(value);
  }
  const bytes = new Uint8Array(length);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.length;
  }
  return bytes;
}

function trimSlash(value: string): string {
  return value.replace(/\/+$/, "");
}

function assertLength(label: string, value: Uint8Array, length: number): void {
  if (value.length !== length) throw new Error(`${label} must contain ${length} bytes`);
}

function equal(left: Uint8Array, right: Uint8Array): boolean {
  if (left.length !== right.length) return false;
  let difference = 0;
  for (let index = 0; index < left.length; index += 1) difference |= left[index] ^ right[index];
  return difference === 0;
}

function asError(value: unknown): Error {
  return value instanceof Error ? value : new Error(String(value));
}
