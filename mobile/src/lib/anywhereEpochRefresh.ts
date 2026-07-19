import type { StoredAnywhereCredentials } from "./transport";
import { deriveDeviceWrapKey } from "./anywhereCrypto";
import { fromBase64Url } from "./anywhereApi";
import {
  bytesFromHex,
  bytesToHex,
  decodeEnvelope,
  openEnvelope,
} from "./transport/anywhereEnvelope";

export interface AnywhereCurrentDeviceWrap {
  version?: 1;
  epoch: number;
  device_wrap_envelope: string;
  signing_public_key: string;
  exchange_public_key: string;
}

/** Validate and decrypt the current device wrap before atomically promoting local credentials. */
export function promoteCurrentDeviceWrap(
  credentials: StoredAnywhereCredentials,
  current: AnywhereCurrentDeviceWrap,
): StoredAnywhereCredentials {
  if (!Number.isSafeInteger(current.epoch) || current.epoch < 1) {
    throw new Error("Forge Anywhere returned an invalid key epoch");
  }
  if (current.epoch === credentials.keyEpoch) return credentials;
  if (current.epoch < credentials.keyEpoch) {
    throw new Error("Forge Anywhere returned an older key epoch");
  }

  const envelopeBytes = fromBase64Url(current.device_wrap_envelope);
  const envelope = decodeEnvelope(envelopeBytes);
  const accountId = bytesFromHex(credentials.accountIdHex);
  if (
    envelope.metadata.kind !== 5
    || envelope.metadata.recipientKind !== 1
    || bytesToHex(envelope.metadata.accountId) !== credentials.accountIdHex
    || bytesToHex(envelope.metadata.recipientId) !== credentials.deviceIdHex
    || envelope.metadata.keyEpoch !== current.epoch
  ) {
    throw new Error("Forge Anywhere returned a device key wrap with mismatched routing metadata");
  }

  const senderId = bytesToHex(envelope.metadata.senderDeviceId);
  const signingPublicKey = fromBase64Url(current.signing_public_key);
  const enrolledSigningKey = credentials.signingPublicKeys[senderId];
  if (!enrolledSigningKey || enrolledSigningKey !== bytesToHex(signingPublicKey)) {
    throw new Error("Forge Anywhere key rotation was not signed by an enrolled device");
  }
  const wrapKey = deriveDeviceWrapKey(
    bytesFromHex(credentials.exchangePrivateKeyHex),
    fromBase64Url(current.exchange_public_key),
    accountId,
    current.epoch,
  );
  const opened = openEnvelope(envelopeBytes, wrapKey, signingPublicKey);
  if (opened.plaintext.length !== 32) {
    throw new Error("Forge Anywhere Account Data Key has an invalid length");
  }
  const accountDataKeyHex = bytesToHex(opened.plaintext);
  return {
    ...credentials,
    accountDataKeyHex,
    dataKeyEpochs: {
      ...(credentials.dataKeyEpochs ?? {
        [String(credentials.keyEpoch)]: credentials.accountDataKeyHex,
      }),
      [String(credentials.keyEpoch)]: credentials.accountDataKeyHex,
      [String(current.epoch)]: accountDataKeyHex,
    },
    keyEpoch: current.epoch,
    // Sequence namespaces include the epoch, so a newly installed epoch starts at zero.
    nextSequence: "0",
  };
}
