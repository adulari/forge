import type { AnywhereAuthSession } from "./anywhereApi";
import { base64Url } from "./anywhereApi";
import { generatePendingKeys, type PendingAnywhereKeys } from "./anywhereCrypto";
import {
  cancelPairing,
  createPairing,
  openApprovedPairing,
  pollPairing,
  type PairingCreateRequest,
} from "./anywherePairing";

/** Public fields of the Forge CLI's sign-in on this computer, read by the desktop shell. */
export interface CliAnywhereAccount { account_id: string; github_login: string | null }

export interface CliLinkResult {
  keys: PendingAnywhereKeys;
  auth: AnywhereAuthSession;
  dataKey: Uint8Array;
  epoch: number;
}

type Invoke = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;

export interface CliLinkDeps {
  invoke: Invoke;
  createPairing: typeof createPairing;
  pollPairing: typeof pollPairing;
  cancelPairing: typeof cancelPairing;
  openApprovedPairing: typeof openApprovedPairing;
  generateKeys: () => PendingAnywhereKeys;
  sleep: (ms: number) => Promise<void>;
}

const defaultDeps: Omit<CliLinkDeps, "invoke"> = {
  createPairing,
  pollPairing,
  cancelPairing,
  openApprovedPairing,
  generateKeys: generatePendingKeys,
  sleep: (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
};

const POLL_ATTEMPTS = 40;
const POLL_INTERVAL_MS = 500;

function bareChallenge(challenge: string): string {
  const trimmed = challenge.trim();
  if (!trimmed.startsWith("forge-anywhere://pair?")) return trimmed;
  return new URL(trimmed).searchParams.get("challenge") ?? "";
}

/**
 * Enrolls this desktop app as a new device of the account the local Forge CLI is signed in to.
 * The app makes its own keys and pairing request; the CLI approves it as an existing device would.
 * Returns null when the CLI is not signed in, or is signed in to another account than
 * `expectedAccountId` (an expired sign-in this app still holds).
 */
export async function linkWithLocalCli(
  serviceUrl: string,
  deviceName: string,
  deps: Partial<CliLinkDeps> & { invoke: Invoke },
  expectedAccountId?: string,
): Promise<CliLinkResult | null> {
  const d = { ...defaultDeps, ...deps };
  const account = await d.invoke<CliAnywhereAccount | null>("forge_anywhere_cli_account");
  if (!account) return null;
  if (expectedAccountId && account.account_id !== expectedAccountId.toLowerCase()) return null;

  const keys = d.generateKeys();
  const request: PairingCreateRequest = {
    version: 1,
    device_name: deviceName,
    signing_public_key: base64Url(keys.signingPublicKey),
    exchange_public_key: base64Url(keys.exchangePublicKey),
  };
  const created = await d.createPairing(serviceUrl, request);
  try {
    await d.invoke("approve_anywhere_pairing_with_cli", {
      challenge: bareChallenge(created.challenge),
      signingPublicKey: request.signing_public_key,
    });
    for (let attempt = 0; attempt < POLL_ATTEMPTS; attempt += 1) {
      const result = await d.pollPairing(serviceUrl, created.pairing_id, created.pairing_token);
      if (result.status === "denied") throw new Error("The Forge CLI approval was denied");
      if (result.status === "approved") {
        if (result.account_id !== account.account_id) {
          throw new Error("The approval came from a different Forge Anywhere account. Nothing was installed.");
        }
        const opened = d.openApprovedPairing(result, keys.exchangePrivateKey);
        return {
          keys,
          dataKey: opened.accountDataKey,
          epoch: opened.epoch,
          auth: {
            version: 1,
            account_id: result.account_id,
            device_id: result.device_id,
            github_login: account.github_login ?? "",
            access_token: result.access_token,
            refresh_token: result.refresh_token,
            access_expires_at_ms: result.access_expires_at_ms,
            new_account: false,
          },
        };
      }
      await d.sleep(POLL_INTERVAL_MS);
    }
    throw new Error("The Forge CLI approval did not arrive in time");
  } catch (reason) {
    await d.cancelPairing(serviceUrl, created.pairing_id, created.pairing_token).catch(() => undefined);
    throw reason;
  }
}
