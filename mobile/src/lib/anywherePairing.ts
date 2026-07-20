import { sha256 } from "@noble/hashes/sha2.js";

import { AnywhereApiError, anywhereRequest, base64Url, fromBase64Url } from "./anywhereApi";
import { deriveDeviceWrapKey, makeKeyWrap } from "./anywhereCrypto";
import type { StoredAnywhereCredentials } from "./transport";
import { bytesFromHex, bytesToHex, decodeEnvelope, openEnvelope } from "./transport/anywhereEnvelope";

export interface PairingChallenge { version: 1; pairing_id: string; exchange_public_key: string; expires_at_ms: number; service_origin: string }
export interface PairingCapability { supported: boolean; message: string }
export interface PairingDetails { version: 1; pairing_id: string; device_id: string; device_name: string; signing_public_key: string; exchange_public_key: string; expires_at_ms: number }
export interface PairingInbox { version: 1; pairings: PairingDetails[] }
export interface PairingApproval { version: 1; epoch: number; device_wrap_envelope: string }
export interface PairingCreateRequest { version: 1; device_name: string; signing_public_key: string; exchange_public_key: string }
export interface PairingCreateResponse { version: 1; pairing_id: string; pairing_token: string; expires_at_ms: number; challenge: string }
export type PairingPollResponse =
  | { version: 1; status: "pending"; expires_at_ms: number }
  | { version: 1; status: "denied" }
  | { version: 1; status: "approved"; account_id: string; device_id: string; access_token: string; refresh_token: string; access_expires_at_ms: number; epoch: number; device_wrap_envelope: string; signing_public_key: string; exchange_public_key: string };

export class PairingPollRateLimitError extends Error {
  constructor(readonly retryAfterMs: number) {
    super("Device approval is busy. Forge will keep checking automatically.");
    this.name = "PairingPollRateLimitError";
  }
}

export function parsePairingChallenge(value: string, serviceUrl: string, now = Date.now()): PairingChallenge {
  let encoded = value.trim();
  if (encoded.startsWith("forge-anywhere://pair?")) encoded = new URL(encoded).searchParams.get("challenge") ?? "";
  let challenge: PairingChallenge;
  try { challenge = JSON.parse(new TextDecoder().decode(decodeBase64(encoded))) as PairingChallenge; }
  catch { throw new Error("QR code is not a Forge Anywhere pairing challenge"); }
  const expectedOrigin = new URL(serviceUrl).origin;
  if (challenge.version !== 1 || !isOpaque32(challenge.pairing_id)
    || !/^[A-Za-z0-9_-]{43}$/.test(challenge.exchange_public_key)
    || challenge.service_origin !== expectedOrigin) throw new Error("Pairing challenge is invalid for this service");
  if (!Number.isSafeInteger(challenge.expires_at_ms) || challenge.expires_at_ms <= now || challenge.expires_at_ms > now + 10 * 60_000) throw new Error("Pairing challenge has expired");
  return challenge;
}

export async function pairingCapability(serviceUrl: string, token: string, fetcher: typeof fetch = fetch): Promise<PairingCapability> {
  const response = await fetcher(new URL("/v1/pairings", serviceUrl), { method: "OPTIONS", headers: { authorization: `Bearer ${token}` }, cache: "no-store" });
  if (response.status === 404 || response.status === 405 || response.status === 501) return { supported: false, message: "Secure QR enrollment is not enabled by this Forge Anywhere service yet." };
  if (!response.ok) throw new Error(`Pairing capability check failed (${response.status})`);
  return { supported: true, message: "This service supports short-lived QR enrollment." };
}

export async function pairingDetails(serviceUrl: string, token: string, challenge: PairingChallenge): Promise<PairingDetails> {
  const details = await anywhereRequest<PairingDetails>(serviceUrl, `/v1/pairings/${challenge.pairing_id}`, { cache: "no-store" }, token);
  validatePairingDetails(details, challenge);
  return details;
}

export async function createPairing(serviceUrl: string, request: PairingCreateRequest): Promise<PairingCreateResponse> {
  const created = await anywhereRequest<PairingCreateResponse>(serviceUrl, "/v1/pairings", {
    method: "POST",
    body: JSON.stringify(request),
  });
  return validateCreatedPairing(created, request, serviceUrl);
}

/** Creates an account-bound request that enrolled clients discover automatically. */
export async function createEnrollmentRequest(
  serviceUrl: string,
  token: string,
  request: PairingCreateRequest,
): Promise<PairingCreateResponse> {
  const created = await anywhereRequest<PairingCreateResponse>(serviceUrl, "/v1/enrollment-requests", {
    method: "POST",
    body: JSON.stringify(request),
  }, token);
  return validateCreatedPairing(created, request, serviceUrl);
}

export async function cancelPairing(
  serviceUrl: string,
  pairingId: string,
  pairingToken: string,
): Promise<void> {
  await anywhereRequest(serviceUrl, `/v1/pairings/${pairingId}/cancel`, {
    method: "POST",
  }, pairingToken);
}

function validateCreatedPairing(
  created: PairingCreateResponse,
  request: PairingCreateRequest,
  serviceUrl: string,
): PairingCreateResponse {
  if (created.version !== 1 || !isOpaque32(created.pairing_id) || !isOpaque32(created.pairing_token)
    || !Number.isSafeInteger(created.expires_at_ms)) {
    throw new Error("Forge Anywhere returned an invalid pairing ticket");
  }
  const challenge = parsePairingChallenge(created.challenge, serviceUrl);
  if (challenge.pairing_id !== created.pairing_id
    || challenge.expires_at_ms !== created.expires_at_ms
    || challenge.exchange_public_key !== request.exchange_public_key) {
    throw new Error("Forge Anywhere returned a mismatched pairing ticket");
  }
  return created;
}

export async function pollPairing(serviceUrl: string, pairingId: string, pairingToken: string, fetcher: typeof fetch = fetch): Promise<PairingPollResponse> {
  if (!isOpaque32(pairingId) || !isOpaque32(pairingToken)) throw new Error("Forge Anywhere pairing ticket is invalid");
  const response = await fetcher(new URL(`/v1/pairings/${pairingId}/poll`, serviceUrl), {
    headers: { authorization: `Bearer ${pairingToken}`, accept: "application/json" },
    cache: "no-store",
  });
  if (!response.ok) {
    if (response.status === 404 || response.status === 410) throw new Error("Device approval request expired");
    if (response.status === 429) throw new PairingPollRateLimitError(retryAfterMilliseconds(response.headers.get("retry-after")));
    throw new Error(`Device approval could not be checked (${response.status})`);
  }
  const result = await response.json() as PairingPollResponse;
  if (result.version !== 1 || !["pending", "approved", "denied"].includes(result.status)) {
    throw new Error("Forge Anywhere returned an invalid pairing result");
  }
  return result;
}

function retryAfterMilliseconds(value: string | null): number {
  if (!value) return 60_000;
  const seconds = Number(value);
  if (Number.isFinite(seconds) && seconds >= 0) return Math.max(1_000, Math.ceil(seconds * 1_000));
  const date = Date.parse(value);
  return Number.isFinite(date) ? Math.max(1_000, date - Date.now()) : 60_000;
}

export async function listPairings(serviceUrl: string, token: string): Promise<PairingDetails[]> {
  const inbox = await anywhereRequest<PairingInbox>(serviceUrl, "/v1/pairings", { cache: "no-store" }, token);
  if (inbox.version !== 1 || !Array.isArray(inbox.pairings)) {
    throw new Error("Forge Anywhere returned an invalid approval inbox");
  }
  const now = Date.now();
  return inbox.pairings.filter((details) => {
    try {
      validatePairingDetails(details, challengeFromDetails(details, serviceUrl));
      return details.expires_at_ms > now;
    } catch {
      return false;
    }
  });
}

export async function denyPairing(serviceUrl: string, token: string, pairingId: string): Promise<void> {
  try {
    await anywhereRequest(serviceUrl, `/v1/pairings/${pairingId}/deny`, {
      method: "POST",
      headers: { "Idempotency-Key": pairingId },
      body: JSON.stringify({ version: 1 }),
    }, token);
  } catch (reason) {
    if (reason instanceof AnywhereApiError && reason.status === 404) {
      throw new Error("Device approval request expired");
    }
    throw reason;
  }
}

export function challengeFromDetails(details: PairingDetails, serviceUrl: string): PairingChallenge {
  return {
    version: 1,
    pairing_id: details.pairing_id,
    exchange_public_key: details.exchange_public_key,
    expires_at_ms: details.expires_at_ms,
    service_origin: new URL(serviceUrl).origin,
  };
}

/** Six digits derived from the authenticated transcript. This is display-only, not a secret. */
export function pairingSafetyCode(
  challenge: PairingChallenge,
  signingPublicKey: string,
  accountIdHex: string,
): string {
  const service = new TextEncoder().encode(challenge.service_origin);
  const transcript = concat(
    new TextEncoder().encode("forge-anywhere/v1/pairing-safety-code\0"),
    fromBase64Url(challenge.pairing_id),
    fromBase64Url(challenge.exchange_public_key),
    fromBase64Url(signingPublicKey),
    u64(BigInt(challenge.expires_at_ms)),
    u32(service.length),
    service,
    bytesFromHex(accountIdHex),
  );
  const digest = sha256(transcript);
  const value = new DataView(digest.buffer, digest.byteOffset, digest.byteLength).getUint32(0, false) % 1_000_000;
  return `${Math.floor(value / 1_000).toString().padStart(3, "0")} ${(value % 1_000).toString().padStart(3, "0")}`;
}

/** Authenticate and open the approved device wrap before a claimant installs account state. */
export function openApprovedPairing(
  result: Extract<PairingPollResponse, { status: "approved" }>,
  claimantExchangePrivateKey: Uint8Array,
): { accountDataKey: Uint8Array; epoch: number } {
  const encoded = fromBase64Url(result.device_wrap_envelope);
  const envelope = decodeEnvelope(encoded);
  if (envelope.metadata.kind !== 5 || envelope.metadata.recipientKind !== 1
    || bytesToHex(envelope.metadata.accountId) !== result.account_id
    || bytesToHex(envelope.metadata.recipientId) !== result.device_id
    || envelope.metadata.keyEpoch !== result.epoch) {
    throw new Error("Approved pairing wrap has mismatched routing metadata");
  }
  const accountId = bytesFromHex(result.account_id);
  const wrapKey = deriveDeviceWrapKey(
    claimantExchangePrivateKey,
    fromBase64Url(result.exchange_public_key),
    accountId,
    result.epoch,
  );
  const opened = openEnvelope(encoded, wrapKey, fromBase64Url(result.signing_public_key));
  if (opened.plaintext.length !== 32) throw new Error("Approved pairing Account Data Key has an invalid length");
  return { accountDataKey: opened.plaintext, epoch: result.epoch };
}

export function preparePairingApproval(credentials: StoredAnywhereCredentials, challenge: PairingChallenge, details: PairingDetails, sequence: bigint): PairingApproval {
  validatePairingDetails(details, challenge);
  const accountId = bytesFromHex(credentials.accountIdHex);
  const wrapKey = deriveDeviceWrapKey(bytesFromHex(credentials.exchangePrivateKeyHex), fromBase64Url(details.exchange_public_key), accountId, credentials.keyEpoch);
  const envelope = makeKeyWrap(
    bytesFromHex(credentials.accountDataKeyHex), wrapKey, accountId,
    bytesFromHex(credentials.deviceIdHex), 1, bytesFromHex(details.device_id),
    credentials.keyEpoch, sequence, bytesFromHex(credentials.signingPrivateKeyHex),
  );
  return { version: 1, epoch: credentials.keyEpoch, device_wrap_envelope: base64Url(envelope) };
}

export async function submitPairingApproval(serviceUrl: string, token: string, pairingId: string, approval: PairingApproval): Promise<void> {
  await anywhereRequest(serviceUrl, `/v1/pairings/${pairingId}/approve`, {
    method: "POST",
    headers: { "Idempotency-Key": pairingId },
    body: JSON.stringify(approval),
  }, token);
}

function validatePairingDetails(details: PairingDetails, challenge: PairingChallenge): void {
  if (details.version !== 1 || details.pairing_id !== challenge.pairing_id
    || !/^[0-9a-f]{32}$/.test(details.device_id)
    || details.exchange_public_key !== challenge.exchange_public_key
    || details.expires_at_ms !== challenge.expires_at_ms
    || fromBase64Url(details.exchange_public_key).length !== 32
    || fromBase64Url(details.signing_public_key).length !== 32) {
    throw new Error("Pairing details do not match the scanned challenge");
  }
}

function decodeBase64(value: string): Uint8Array {
  if (!/^[A-Za-z0-9_-]+$/.test(value)) throw new Error("base64url");
  const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
  const bytes: number[] = []; let bits = 0; let count = 0;
  for (const char of value) { bits = bits * 64 + alphabet.indexOf(char); count += 6; if (count >= 8) { count -= 8; bytes.push(Math.floor(bits / 2 ** count) & 255); bits %= 2 ** count; } }
  return new Uint8Array(bytes);
}

function isOpaque32(value: string): boolean {
  try { return fromBase64Url(value).length === 32; } catch { return false; }
}

function u32(value: number): Uint8Array {
  const bytes = new Uint8Array(4);
  new DataView(bytes.buffer).setUint32(0, value, false);
  return bytes;
}

function u64(value: bigint): Uint8Array {
  const bytes = new Uint8Array(8);
  new DataView(bytes.buffer).setBigUint64(0, value, false);
  return bytes;
}

function concat(...parts: Uint8Array[]): Uint8Array {
  const output = new Uint8Array(parts.reduce((total, part) => total + part.length, 0));
  let offset = 0;
  for (const part of parts) { output.set(part, offset); offset += part.length; }
  return output;
}
