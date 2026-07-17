/** Owner/device secret state. Never serialize this into Forge config or ordinary localStorage. */
export interface StoredAnywhereCredentials {
  version: 1;
  accountIdHex: string;
  deviceIdHex: string;
  signingPrivateKeyHex: string;
  exchangePrivateKeyHex: string;
  accountDataKeyHex: string;
  keyEpoch: number;
  accessToken: string;
  refreshToken: string;
  accessExpiresAtMs: number;
  nextSequence: string;
  acceptedSequences: Record<string, string>;
  signingPublicKeys: Record<string, string>;
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
    || typeof parsed.keyEpoch !== "number"
    || typeof parsed.accessToken !== "string"
    || typeof parsed.refreshToken !== "string"
    || typeof parsed.accessExpiresAtMs !== "number"
    || typeof parsed.nextSequence !== "string"
    || typeof parsed.acceptedSequences !== "object"
    || parsed.acceptedSequences == null
    || typeof parsed.signingPublicKeys !== "object"
    || parsed.signingPublicKeys == null
  ) {
    throw new Error("stored Forge Anywhere credentials are invalid");
  }
  return parsed as StoredAnywhereCredentials;
}
