/** Owner/device secret state. Never serialize this into Forge config or ordinary localStorage. */
export interface StoredAnywhereCredentials {
  version: 1;
  serviceUrl?: string;
  githubLogin?: string;
  accountIdHex: string;
  deviceIdHex: string;
  signingPrivateKeyHex: string;
  exchangePrivateKeyHex: string;
  accountDataKeyHex: string;
  /** Historical epoch keys remain device secrets and stay in the protected credential store. */
  dataKeyEpochs?: Record<string, string>;
  keyEpoch: number;
  accessToken: string;
  refreshToken: string;
  accessExpiresAtMs: number;
  nextSequence: string;
  acceptedSequences: Record<string, string | string[]>;
  signingPublicKeys: Record<string, string>;
  /** A proven 401 keeps zero-knowledge keys but disables this device until it is approved again. */
  reauthenticationRequired?: boolean;
  /** This device successfully verified or used the account Recovery Kit. */
  recoveryKitVerified?: boolean;
  /** Exact protected retry journal for an ambiguously acknowledged atomic device revocation. */
  pendingDeviceRevocation?: PendingDeviceRevocation;
}

export interface PendingDeviceRevocation {
  targetDeviceId: string;
  idempotencyKey: string;
  epoch: number;
  request: {
    epoch: number;
    recovery_wrap_envelope: string;
    device_wraps: { device_id: string; envelope: string }[];
  };
  nextCredentials: StoredAnywhereCredentials;
}

export interface AnywhereCredentialStore {
  load(): Promise<StoredAnywhereCredentials | null>;
  save(credentials: StoredAnywhereCredentials): Promise<void>;
  clear(): Promise<void>;
}

export function parseStoredCredentials(value: string): StoredAnywhereCredentials {
  const parsed = JSON.parse(value) as Partial<StoredAnywhereCredentials>;
  if (
    parsed.version !== 1
    || typeof parsed.accountIdHex !== "string"
    || typeof parsed.deviceIdHex !== "string"
    || typeof parsed.signingPrivateKeyHex !== "string"
    || typeof parsed.exchangePrivateKeyHex !== "string"
    || typeof parsed.accountDataKeyHex !== "string"
    || (parsed.dataKeyEpochs !== undefined && (typeof parsed.dataKeyEpochs !== "object" || parsed.dataKeyEpochs == null))
    || typeof parsed.keyEpoch !== "number"
    || typeof parsed.accessToken !== "string"
    || typeof parsed.refreshToken !== "string"
    || typeof parsed.accessExpiresAtMs !== "number"
    || typeof parsed.nextSequence !== "string"
    || typeof parsed.acceptedSequences !== "object"
    || parsed.acceptedSequences == null
    || !Object.values(parsed.acceptedSequences).every((entry) => typeof entry === "string"
      || (Array.isArray(entry) && entry.every((sequence) => typeof sequence === "string")))
    || typeof parsed.signingPublicKeys !== "object"
    || parsed.signingPublicKeys == null
    || (parsed.serviceUrl !== undefined && typeof parsed.serviceUrl !== "string")
    || (parsed.githubLogin !== undefined && typeof parsed.githubLogin !== "string")
    || (parsed.recoveryKitVerified !== undefined && typeof parsed.recoveryKitVerified !== "boolean")
    || (parsed.reauthenticationRequired !== undefined && typeof parsed.reauthenticationRequired !== "boolean")
    || (parsed.pendingDeviceRevocation !== undefined && !isPendingRevocation(parsed.pendingDeviceRevocation))
  ) {
    throw new Error("stored Forge Anywhere credentials are invalid");
  }
  return parsed as StoredAnywhereCredentials;
}

function isPendingRevocation(value: unknown): value is PendingDeviceRevocation {
  if (!isRecord(value) || !isRecord(value.request) || !isRecord(value.nextCredentials)) return false;
  const wraps = value.request.device_wraps;
  return typeof value.targetDeviceId === "string"
    && typeof value.idempotencyKey === "string"
    && Number.isSafeInteger(value.epoch)
    && value.request.epoch === value.epoch
    && typeof value.request.recovery_wrap_envelope === "string"
    && Array.isArray(wraps)
    && wraps.every((wrap) => isRecord(wrap)
      && typeof wrap.device_id === "string"
      && typeof wrap.envelope === "string")
    && value.nextCredentials.pendingDeviceRevocation === undefined
    && value.nextCredentials.keyEpoch === value.epoch;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
