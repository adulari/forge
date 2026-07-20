import { secureRandomBytes } from "./secureRandom";

export const DEFAULT_ANYWHERE_SERVICE_URL = "https://app.forge.adulari.dev";

export interface AnywhereDeviceFlow {
  version: 1;
  device_code: string;
  user_code: string;
  verification_uri: string;
  expires_in: number;
  interval: number;
}

export interface AnywhereAuthSession {
  version: 1;
  account_id: string;
  device_id: string;
  github_login: string;
  access_token: string;
  refresh_token: string;
  access_expires_at_ms: number;
  new_account: boolean;
  recovery_wrap_envelope?: string;
  recovery_wrap_signing_public_key?: string;
}

export interface AnywhereCapabilities {
  version: 1;
  service_version: string;
  protocol_version: number;
  minimum_client_version: string;
  maximum_client_major: number;
  ready: boolean;
  features: {
    account_bound_enrollment: boolean;
    passkey_prf_recovery: boolean;
    seven_day_clean_reset: boolean;
    recovery_kit_v2: boolean;
    legacy_recovery_kit_v1: boolean;
  };
}

export async function preflightAnywhere(serviceUrl: string): Promise<AnywhereCapabilities> {
  const capabilities = await anywhereRequest<AnywhereCapabilities>(
    serviceUrl,
    "/v1/capabilities",
    { cache: "no-store" },
  );
  if (capabilities.version !== 1 || capabilities.protocol_version !== 2
    || capabilities.maximum_client_major < 2) {
    throw new Error("Update Forge before setting up Forge Anywhere.");
  }
  if (!capabilities.ready) {
    throw new Error("Forge Anywhere is temporarily unavailable. Your local Forge data is unaffected.");
  }
  if (!capabilities.features.account_bound_enrollment
    || !capabilities.features.recovery_kit_v2) {
    throw new Error("Forge Anywhere is being updated. Try setup again shortly.");
  }
  return capabilities;
}

export interface AnywhereAccountStatus {
  version: 1;
  entitlement: string;
  trial_ends_at: string | null;
  active_hosts: number;
  devices: number;
  storage_used_bytes: number;
  storage_limit_bytes: number;
  pending_reset: AnywherePendingReset | null;
}

export interface AnywherePendingReset {
  requested_at_ms: number;
  executes_at_ms: number;
  cancelable: boolean;
}

export interface AnywhereResetStatus {
  version: 1;
  pending_reset: AnywherePendingReset | null;
}

export type AnywhereBillingPeriod = "monthly" | "annual";

export interface AnywhereSubscription {
  version: 1;
  state: string;
  subscription_status: string | null;
  paid_through: number | null;
  trial_ends_at: number | null;
  grace_ends_at: number | null;
  read_only_ends_at: number | null;
  retention_ends_at: number | null;
  cancel_at_period_end: boolean;
}

export interface AnywhereRecoveryWrap {
  version: 1;
  epoch: number;
  recovery_wrap_envelope: string;
  signing_public_key: string;
}

export interface AnywhereCheckoutSession {
  version: 1;
  checkout_url: string;
}

export interface AnywherePortalSession {
  version: 1;
  portal_url: string;
}

export interface AnywhereHost {
  id: string;
  device_id: string;
  name: string;
  created_at: string;
  last_heartbeat_at: string | null;
  online?: boolean;
}

export interface AnywhereDevice {
  id: string;
  name: string;
  created_at: string;
  last_seen_at: string | null;
  signing_public_key: string;
  exchange_public_key: string;
}

interface ApiErrorBody {
  code?: string;
  message?: string;
  error?: { code?: string; message?: string };
}

export class AnywhereApiError extends Error {
  constructor(
    readonly status: number,
    readonly code: string,
    message: string,
    readonly retryAfterMs?: number,
  ) {
    super(message);
    this.name = "AnywhereApiError";
  }
}

type AnywhereUnauthorizedListener = (rejectedAccessToken: string) => void;
const unauthorizedListeners = new Set<AnywhereUnauthorizedListener>();

export function isAnywhereSessionInvalid(reason: unknown): reason is AnywhereApiError {
  return reason instanceof AnywhereApiError && reason.status === 401;
}

/** Observe authenticated 401s without ever putting bearer credentials in URLs or logs. */
export function observeAnywhereUnauthorized(listener: AnywhereUnauthorizedListener): () => void {
  unauthorizedListeners.add(listener);
  return () => unauthorizedListeners.delete(listener);
}

export async function anywhereRequest<T>(
  serviceUrl: string,
  path: string,
  init: RequestInit = {},
  accessToken?: string,
): Promise<T> {
  const response = await fetch(`${serviceUrl.replace(/\/$/, "")}${path}`, {
    ...init,
    headers: {
      accept: "application/json",
      ...(init.body ? { "content-type": "application/json" } : {}),
      ...(accessToken ? { authorization: `Bearer ${accessToken}` } : {}),
      ...init.headers,
    },
  });
  if (response.status === 202) return undefined as T;
  if (!response.ok) {
    let body: ApiErrorBody = {};
    try { body = await response.json() as ApiErrorBody; } catch { /* empty/non-JSON error */ }
    const detail = body.error ?? body;
    const error = new AnywhereApiError(
      response.status,
      detail.code ?? "request_failed",
      detail.message ?? `Forge Anywhere request failed (${response.status})`,
      response.status === 429 ? retryAfterMilliseconds(response.headers.get("retry-after")) : undefined,
    );
    if (response.status === 401 && accessToken) {
      for (const listener of unauthorizedListeners) listener(accessToken);
    }
    throw error;
  }
  if (response.status === 204) return undefined as T;
  return await response.json() as T;
}

function retryAfterMilliseconds(value: string | null): number {
  if (!value) return 60_000;
  const seconds = Number(value);
  if (Number.isFinite(seconds) && seconds >= 0) return Math.max(1_000, Math.ceil(seconds * 1_000));
  const date = Date.parse(value);
  return Number.isFinite(date) ? Math.max(1_000, date - Date.now()) : 60_000;
}

export function idempotencyKey(): string {
  return Array.from(secureRandomBytes(16), (byte) =>
    byte.toString(16).padStart(2, "0"),
  ).join("");
}

export function base64Url(bytes: Uint8Array): string {
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

export function fromBase64Url(value: string): Uint8Array {
  if (!/^[A-Za-z0-9_-]*$/.test(value) || value.length % 4 === 1) throw new Error("invalid base64url");
  const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
  const output = new Uint8Array(Math.floor(value.length * 6 / 8));
  let bits = 0;
  let bitCount = 0;
  let offset = 0;
  for (const character of value) {
    const digit = alphabet.indexOf(character);
    if (digit < 0) throw new Error("invalid base64url");
    bits = (bits << 6) | digit;
    bitCount += 6;
    if (bitCount >= 8) {
      bitCount -= 8;
      output[offset] = (bits >>> bitCount) & 0xff;
      offset += 1;
      bits &= (1 << bitCount) - 1;
    }
  }
  if (bitCount > 0 && bits !== 0) throw new Error("non-canonical base64url");
  return output;
}
