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

export interface AnywhereAccountStatus {
  version: 1;
  entitlement: string;
  trial_ends_at: string | null;
  active_hosts: number;
  devices: number;
  storage_used_bytes: number;
  storage_limit_bytes: number;
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
}

export interface AnywhereDevice {
  id: string;
  name: string;
  created_at: string;
  last_seen_at: string | null;
  signing_public_key: string;
  exchange_public_key: string;
}

interface ApiErrorBody { code?: string; message?: string }

export class AnywhereApiError extends Error {
  constructor(readonly status: number, readonly code: string, message: string) {
    super(message);
    this.name = "AnywhereApiError";
  }
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
    throw new AnywhereApiError(
      response.status,
      body.code ?? "request_failed",
      body.message ?? `Forge Anywhere request failed (${response.status})`,
    );
  }
  if (response.status === 204) return undefined as T;
  return await response.json() as T;
}

export function idempotencyKey(): string {
  return Array.from(crypto.getRandomValues(new Uint8Array(16)), (byte) =>
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
