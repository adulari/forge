import { decodeEnvelope, openEnvelope, bytesFromHex, bytesToHex } from "./transport/anywhereEnvelope";
import { fromBase64Url } from "./anywhereApi";

export interface ReplayShare<T = unknown> {
  version: 1;
  session_id: string;
  created_at_ms: number;
  expires_at_ms: number;
  replay: T;
}

export interface ReplayShareRequest {
  serviceUrl: string;
  shareId: string;
  href: string;
  fetcher?: typeof fetch;
  resolveSigningKey?(senderDeviceIdHex: string): Uint8Array | null;
  now?: () => number;
}

export interface ReplayFragment { key: Uint8Array; signingPublicKey: Uint8Array }

/** Read replay key material locally. URL fragments are never returned to network callers. */
export function replayFragmentFromHref(href: string): ReplayFragment {
  const url = new URL(href);
  if (url.search.length > 0) throw new Error("Replay links cannot contain query parameters");
  const fragment = new URLSearchParams(url.hash.slice(1));
  if ([...fragment.keys()].some((key) => key !== "key" && key !== "signing")
    || fragment.getAll("key").length !== 1 || fragment.getAll("signing").length !== 1) {
    throw new Error("Replay link fragment is invalid");
  }
  const key = fromBase64Url(fragment.get("key") ?? "");
  if (key.length !== 32) throw new Error("Replay link key must contain 32 bytes");
  const signingPublicKey = fromBase64Url(fragment.get("signing") ?? "");
  if (signingPublicKey.length !== 32) throw new Error("Replay signing key must contain 32 bytes");
  return { key, signingPublicKey };
}

export function replayKeyFromHref(href: string): Uint8Array { return replayFragmentFromHref(href).key; }

export async function retrieveReplayShare<T = unknown>(request: ReplayShareRequest): Promise<ReplayShare<T>> {
  if (!/^[0-9a-f]{32}$/.test(request.shareId)) throw new Error("Replay share id is invalid");
  const page = new URL(request.href);
  const service = new URL(request.serviceUrl);
  if (page.origin !== service.origin) throw new Error("Replay link origin does not match Forge Anywhere");
  const fragment = replayFragmentFromHref(request.href);
  const downloadUrl = new URL(`/v1/shares/${request.shareId}`, service);
  const response = await (request.fetcher ?? fetch)(downloadUrl.toString(), {
    method: "GET",
    headers: { accept: "application/vnd.forge-anywhere" },
    cache: "no-store",
    referrerPolicy: "no-referrer",
  });
  if (!response.ok) throw new Error(response.status === 404 ? "Replay link is missing, expired, or revoked" : `Replay download failed (${response.status})`);
  const contentType = response.headers.get("content-type")?.split(";", 1)[0];
  if (contentType !== "application/vnd.forge-anywhere") throw new Error("Replay response has an unexpected content type");
  const bytes = new Uint8Array(await response.arrayBuffer());
  const decoded = decodeEnvelope(bytes);
  if (decoded.metadata.kind !== 7 || decoded.metadata.recipientKind !== 4 || decoded.metadata.keyEpoch !== 0
    || bytesToHex(decoded.metadata.recipientId) !== request.shareId) {
    throw new Error("Replay envelope routing metadata does not match this link");
  }
  const trusted = request.resolveSigningKey?.(bytesToHex(decoded.metadata.senderDeviceId)) ?? null;
  if (trusted != null && bytesToHex(trusted) !== bytesToHex(fragment.signingPublicKey)) {
    throw new Error("Replay signing key does not match the trusted sender");
  }
  const opened = openEnvelope(bytes, fragment.key, fragment.signingPublicKey);
  let value: unknown;
  try { value = JSON.parse(new TextDecoder().decode(opened.plaintext)); }
  catch { throw new Error("Replay payload is not valid JSON"); }
  if (!isReplayShare(value)) throw new Error("Replay payload does not match the v1 format");
  const now = (request.now ?? Date.now)();
  if (value.expires_at_ms <= now || value.expires_at_ms <= value.created_at_ms) throw new Error("Replay link has expired");
  if (BigInt(value.created_at_ms) !== decoded.metadata.createdAtMs) throw new Error("Replay creation time does not match its envelope");
  return value as ReplayShare<T>;
}

export function trustedReplaySigner(keys: Record<string, string>, senderDeviceIdHex: string): Uint8Array | null {
  const value = keys[senderDeviceIdHex];
  if (value == null) return null;
  try {
    const key = bytesFromHex(value);
    return key.length === 32 ? key : null;
  } catch { return null; }
}

function isReplayShare(value: unknown): value is ReplayShare {
  if (typeof value !== "object" || value == null || Array.isArray(value)) return false;
  const record = value as Record<string, unknown>;
  return Object.keys(record).every((key) => ["version", "session_id", "created_at_ms", "expires_at_ms", "replay"].includes(key))
    && record.version === 1 && typeof record.session_id === "string" && record.session_id.length > 0
    && Number.isSafeInteger(record.created_at_ms) && Number.isSafeInteger(record.expires_at_ms)
    && "replay" in record;
}
