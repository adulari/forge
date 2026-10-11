// Which (host, device token, key) triples the daemon has been told about. Pure so the "does this
// registration need refreshing" rule is unit-tested; push.ios.ts persists it. It holds only a
// fingerprint of the public key, never the key material.
import type { PushKeys } from "./pushE2e";

export type RegistrationLedger = Record<string, string>;

function fingerprint(token: string, keys: PushKeys | null): string {
  return `${token}|${keys ? keys.p256dh : "-"}`;
}

export function pushRegistrationDue(
  ledger: RegistrationLedger,
  baseUrl: string,
  token: string,
  keys: PushKeys | null,
): boolean {
  return ledger[baseUrl] !== fingerprint(token, keys);
}

export function withRegistration(
  ledger: RegistrationLedger,
  baseUrl: string,
  token: string,
  keys: PushKeys | null,
): RegistrationLedger {
  return { ...ledger, [baseUrl]: fingerprint(token, keys) };
}

export function withoutRegistration(ledger: RegistrationLedger, baseUrl: string): RegistrationLedger {
  const { [baseUrl]: _removed, ...rest } = ledger;
  return rest;
}
