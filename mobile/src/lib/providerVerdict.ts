import type { ProviderRow } from "./api";

/** A provider whose key the provider itself refused. Null for every other row. The row is the
 * daemon's `key_verdict` (additive on /api/providers), so an older daemon simply has none. */
export function keyVerdict(provider: ProviderRow) {
  return provider.key_verdict ?? null;
}

/** "Key expired — renew" / "Key rejected" / "Out of quota — top up", from the daemon. */
export function verdictHeadline(provider: ProviderRow): string | null {
  return keyVerdict(provider)?.headline ?? null;
}

/** The provider's own reason plus how long it has been refusing, for the line under the row. */
export function verdictDetail(provider: ProviderRow, nowSeconds: number): string | null {
  const verdict = keyVerdict(provider);
  if (!verdict) return null;
  const days = Math.floor(Math.max(0, nowSeconds - verdict.first_seen) / 86_400);
  const since = days >= 1 ? `since ${days}d ago` : "since today";
  return verdict.reason ? `${verdict.reason} · ${since}` : since;
}
