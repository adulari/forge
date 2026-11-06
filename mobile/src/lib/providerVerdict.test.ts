import { describe, expect, it } from "vitest";

import type { ProviderRow } from "./api";
import { verdictDetail, verdictHeadline } from "./providerVerdict";

const row = (key_verdict: ProviderRow["key_verdict"]): ProviderRow => ({
  id: "opencode",
  label: "OpenCode",
  kind: "api_key",
  enabled: true,
  configured: true,
  auth_status: "configured",
  keyless: false,
  env_var: "OPENCODE_API_KEY",
  environment_key_present: false,
  stored_key_fingerprints: ["…abcd"],
  free: false,
  endpoint: null,
  azure_resource: null,
  azure_api_version: null,
  models: [],
  accounts: [],
  login_command: null,
  installed: null,
  version: null,
  serving: null,
  restart_required: false,
  key_verdict,
});

describe("provider key verdict", () => {
  it("reads the daemon's headline and shows the reason with its age", () => {
    const expired = row({
      status: "expired",
      headline: "Key expired — renew",
      reason: "This API key has expired",
      first_seen: 1_000,
      last_seen: 2_000,
    });
    expect(verdictHeadline(expired)).toBe("Key expired — renew");
    expect(verdictDetail(expired, 1_000 + 20 * 86_400)).toBe("This API key has expired · since 20d ago");
    expect(verdictDetail(expired, 1_000 + 60)).toBe("This API key has expired · since today");
  });

  it("is silent for a healthy row and for an older daemon that sends no field", () => {
    expect(verdictHeadline(row(null))).toBeNull();
    expect(verdictDetail(row(undefined), 5)).toBeNull();
  });
});
