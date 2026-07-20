import {
  type AnywhereAuthSession,
  type AnywhereBillingPeriod,
  type AnywhereDevice,
  type AnywhereRecoveryWrap,
  base64Url,
  fromBase64Url,
} from "./anywhereApi";
import {
  deriveDeviceWrapKey,
  deriveRecoveryWrapKey,
  makeKeyWrap,
  openRecoveryWrap,
  recoveryEntropyFromInput,
} from "./anywhereCrypto";
import { secureRandomBytes } from "./secureRandom";
import type { PendingDeviceRevocation, StoredAnywhereCredentials } from "./transport";
import { bytesFromHex, bytesToHex } from "./transport/anywhereEnvelope";

export const DEFAULT_BILLING_PERIOD: AnywhereBillingPeriod = "annual";

export function billingCheckoutBody(period: AnywhereBillingPeriod = DEFAULT_BILLING_PERIOD): {
  billing_period: AnywhereBillingPeriod;
} {
  return { billing_period: period };
}

type RefreshResponse = Pick<AnywhereAuthSession, "access_token" | "refresh_token" | "access_expires_at_ms">;

export async function refreshPendingAnywhereAuth(
  current: AnywhereAuthSession,
  refresh: (refreshToken: string) => Promise<RefreshResponse>,
  now = Date.now(),
): Promise<AnywhereAuthSession> {
  if (current.access_expires_at_ms > now + 30_000) return current;
  const response = await refresh(current.refresh_token);
  return {
    ...current,
    access_token: response.access_token,
    refresh_token: response.refresh_token,
    access_expires_at_ms: response.access_expires_at_ms,
  };
}

export async function refreshAnywhereCredentials(
  current: StoredAnywhereCredentials,
  refresh: (refreshToken: string) => Promise<RefreshResponse>,
  persist: (credentials: StoredAnywhereCredentials) => Promise<void>,
  now = Date.now(),
): Promise<StoredAnywhereCredentials> {
  if (current.accessExpiresAtMs > now + 30_000) return current;
  const response = await refresh(current.refreshToken);
  const next = {
    ...current,
    accessToken: response.access_token,
    refreshToken: response.refresh_token,
    accessExpiresAtMs: response.access_expires_at_ms,
  };
  await persist(next);
  return next;
}

export interface RevokeDeviceRequest {
  epoch: number;
  recovery_wrap_envelope: string;
  device_wraps: { device_id: string; envelope: string }[];
}

export interface PreparedDeviceRevocation {
  epoch: number;
  request: RevokeDeviceRequest;
  nextCredentials: StoredAnywhereCredentials;
}

export function prepareDeviceRevocation(
  credentials: StoredAnywhereCredentials,
  devices: readonly AnywhereDevice[],
  targetDeviceId: string,
  recoveryWords: string,
  currentRecovery: AnywhereRecoveryWrap,
  randomBytes: (length: number) => Uint8Array = secureRandomBytes,
  serviceUrl = "https://app.forge.adulari.dev",
): PreparedDeviceRevocation {
  if (targetDeviceId === credentials.deviceIdHex) throw new Error("Use logout to remove this device");
  if (!devices.some((device) => device.id === targetDeviceId)) throw new Error("That device is no longer enrolled");
  if (currentRecovery.epoch !== credentials.keyEpoch) throw new Error("The account key changed; refresh and try again");

  const recovered = openRecoveryWrap(
    currentRecovery.recovery_wrap_envelope,
    currentRecovery.signing_public_key,
    recoveryWords,
    credentials.accountIdHex,
    serviceUrl,
  );
  if (bytesToHex(recovered.dataKey) !== credentials.accountDataKeyHex) {
    throw new Error("Recovery phrase does not match this account; no device was revoked");
  }

  const epoch = credentials.keyEpoch + 1;
  if (!Number.isSafeInteger(epoch) || epoch > 0xffff_ffff) throw new Error("The account key epoch is exhausted");
  const accountId = bytesFromHex(credentials.accountIdHex);
  const senderDeviceId = bytesFromHex(credentials.deviceIdHex);
  const exchangePrivateKey = bytesFromHex(credentials.exchangePrivateKeyHex);
  const signingPrivateKey = bytesFromHex(credentials.signingPrivateKeyHex);
  const dataKey = randomBytes(32);
  if (dataKey.length !== 32) throw new Error("Secure randomness returned the wrong key length");
  let sequence = BigInt(credentials.nextSequence);

  const deviceWraps = devices
    .filter((device) => device.id !== targetDeviceId)
    .map((device) => {
      const recipientId = bytesFromHex(device.id);
      const wrapKey = deriveDeviceWrapKey(
        exchangePrivateKey,
        fromBase64Url(device.exchange_public_key),
        accountId,
        epoch,
      );
      const envelope = makeKeyWrap(
        dataKey,
        wrapKey,
        accountId,
        senderDeviceId,
        1,
        recipientId,
        epoch,
        sequence,
        signingPrivateKey,
      );
      sequence += 1n;
      return { device_id: device.id, envelope: base64Url(envelope) };
    });

  const recoveryKey = deriveRecoveryWrapKey(
    recoveryEntropyFromInput(recoveryWords, serviceUrl, credentials.accountIdHex),
    accountId,
    epoch,
  );
  const recoveryWrap = makeKeyWrap(
    dataKey,
    recoveryKey,
    accountId,
    senderDeviceId,
    3,
    accountId,
    epoch,
    sequence,
    signingPrivateKey,
  );
  sequence += 1n;
  const accountDataKeyHex = bytesToHex(dataKey);
  return {
    epoch,
    request: {
      epoch,
      recovery_wrap_envelope: base64Url(recoveryWrap),
      device_wraps: deviceWraps,
    },
    nextCredentials: {
      ...credentials,
      accountDataKeyHex,
      dataKeyEpochs: {
        ...(credentials.dataKeyEpochs ?? { [String(credentials.keyEpoch)]: credentials.accountDataKeyHex }),
        [String(epoch)]: accountDataKeyHex,
      },
      keyEpoch: epoch,
      nextSequence: sequence.toString(),
      signingPublicKeys: Object.fromEntries(
        Object.entries(credentials.signingPublicKeys).filter(([deviceId]) => deviceId !== targetDeviceId),
      ),
    },
  };
}

export async function stagePreparedDeviceRevocation(
  current: StoredAnywhereCredentials,
  prepared: PreparedDeviceRevocation,
  targetDeviceId: string,
  stableIdempotencyKey: string,
  persist: (credentials: StoredAnywhereCredentials) => Promise<void>,
): Promise<PendingDeviceRevocation> {
  const pending: PendingDeviceRevocation = {
    targetDeviceId,
    idempotencyKey: stableIdempotencyKey,
    epoch: prepared.epoch,
    request: prepared.request,
    nextCredentials: { ...prepared.nextCredentials, pendingDeviceRevocation: undefined },
  };
  // Consume the wrap sequences before submitting, but keep the current epoch active until the
  // service's atomic transition is known to have committed.
  await persist({ ...current, nextSequence: prepared.nextCredentials.nextSequence, pendingDeviceRevocation: pending });
  return pending;
}

export async function commitPendingDeviceRevocation(
  pending: PendingDeviceRevocation,
  submit: (request: RevokeDeviceRequest, idempotencyKey: string) => Promise<{ epoch: number }>,
  isCommitted: () => Promise<boolean>,
  persist: (credentials: StoredAnywhereCredentials) => Promise<void>,
): Promise<void> {
  try {
    const response = await submit(pending.request, pending.idempotencyKey);
    if (response.epoch !== pending.epoch) throw new Error("The service acknowledged the wrong replacement key epoch");
  } catch (error) {
    // A lost response is indistinguishable from a failed write. Query authoritative state before
    // retaining the journal; retries still use the exact ciphertext and idempotency key.
    const committed = await isCommitted().catch(() => false);
    if (!committed) throw error;
  }
  await persist({ ...pending.nextCredentials, pendingDeviceRevocation: undefined });
}
