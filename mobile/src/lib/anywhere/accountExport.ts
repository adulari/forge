// Honest local account export.
//
// The managed service exposes no export endpoint (there is no `/v1/account/export`), so
// this serializes exactly the account records this device already holds — the same ones
// the Anywhere screens render — and nothing else. Secrets are excluded by construction:
// this module only ever sees public metadata, never the credential store.
import type {
  AnywhereAccountStatus,
  AnywhereDevice,
  AnywhereHost,
  AnywhereSubscription,
} from "../anywhereApi";
import type { AnywherePasskey } from "../anywherePasskeys";

export interface AccountExportInput {
  serviceUrl: string;
  githubLogin?: string;
  accountIdHex: string;
  deviceIdHex: string;
  keyEpoch: number;
  account: AnywhereAccountStatus | null;
  subscription: AnywhereSubscription | null;
  hosts: readonly AnywhereHost[];
  devices: readonly AnywhereDevice[];
  passkeys: readonly AnywherePasskey[];
}

/**
 * A pretty-printed JSON snapshot. `scope: "device-local-snapshot"` is part of the payload
 * so a reader can never mistake it for a server-produced archive of encrypted content.
 */
export function buildAccountExport(input: AccountExportInput, nowMs: number = Date.now()): string {
  return JSON.stringify({
    version: 1,
    kind: "forge-anywhere-account-export",
    scope: "device-local-snapshot",
    note: "Account metadata held by this device. It contains no session content, no encrypted objects, and no key material.",
    exported_at: new Date(nowMs).toISOString(),
    service_url: input.serviceUrl,
    account: {
      account_id: input.accountIdHex,
      github_login: input.githubLogin ?? null,
      this_device_id: input.deviceIdHex,
      key_epoch: input.keyEpoch,
      entitlement: input.account?.entitlement ?? null,
      trial_ends_at: input.account?.trial_ends_at ?? null,
      active_hosts: input.account?.active_hosts ?? null,
      devices: input.account?.devices ?? null,
      storage_used_bytes: input.account?.storage_used_bytes ?? null,
      storage_limit_bytes: input.account?.storage_limit_bytes ?? null,
      pending_reset: input.account?.pending_reset ?? null,
    },
    subscription: input.subscription,
    hosts: input.hosts.map((host) => ({
      id: host.id,
      device_id: host.device_id,
      name: host.name,
      created_at: host.created_at,
      last_heartbeat_at: host.last_heartbeat_at,
      ...(host.connector_version === undefined ? {} : { connector_version: host.connector_version }),
      ...(host.disabled === undefined ? {} : { disabled: host.disabled }),
    })),
    devices: input.devices.map((device) => ({
      id: device.id,
      name: device.name,
      created_at: device.created_at,
      last_seen_at: device.last_seen_at,
      signing_public_key: device.signing_public_key,
      exchange_public_key: device.exchange_public_key,
    })),
    passkeys: input.passkeys.map((passkey) => ({
      id: passkey.id,
      name: passkey.name,
      created_at: passkey.created_at,
      last_used_at: passkey.last_used_at,
    })),
  }, null, 2);
}
