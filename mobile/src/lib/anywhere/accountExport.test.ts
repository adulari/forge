import { describe, expect, it } from "vitest";

import { buildAccountExport, type AccountExportInput } from "./accountExport";

const input: AccountExportInput = {
  serviceUrl: "https://app.example",
  githubLogin: "mkramer",
  accountIdHex: "a".repeat(32),
  deviceIdHex: "b".repeat(32),
  keyEpoch: 3,
  account: {
    version: 1,
    entitlement: "trialing",
    trial_ends_at: "2026-08-01T00:00:00Z",
    active_hosts: 2,
    devices: 3,
    storage_used_bytes: 1234,
    storage_limit_bytes: 5678,
    pending_reset: null,
  },
  subscription: null,
  hosts: [{ id: "h1", device_id: "d1", name: "atlas", created_at: "2026-07-01T00:00:00Z", last_heartbeat_at: "1753000000" }],
  devices: [{ id: "d1", name: "MacBook", created_at: "2026-06-01T00:00:00Z", last_seen_at: null, signing_public_key: "AAAA", exchange_public_key: "BBBB" }],
  passkeys: [{ id: "p1", name: "iCloud", created_at: 1, last_used_at: null }],
};

describe("buildAccountExport", () => {
  it("labels itself a device-local snapshot and carries the records the screens show", () => {
    const parsed = JSON.parse(buildAccountExport(input, 0)) as Record<string, unknown>;
    expect(parsed.scope).toBe("device-local-snapshot");
    expect(parsed.exported_at).toBe("1970-01-01T00:00:00.000Z");
    expect(parsed.hosts).toHaveLength(1);
    expect(parsed.devices).toHaveLength(1);
    expect(parsed.passkeys).toHaveLength(1);
  });

  it("omits optional host fields the service did not return", () => {
    const host = (JSON.parse(buildAccountExport(input, 0)) as { hosts: Record<string, unknown>[] }).hosts[0];
    expect(host).not.toHaveProperty("connector_version");
    expect(host).not.toHaveProperty("disabled");
  });
});
