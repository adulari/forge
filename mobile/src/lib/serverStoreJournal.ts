// The stored server list is one JSON value in a key-value store. A single write is atomic, but a
// pairing is several steps and the process can die between them, and a value that fails to parse
// would otherwise read as "no servers" and unpair the app. Writes go to a staging key first and
// are promoted to the real key after, the key-value equivalent of temp file plus rename; a read
// finishes a promotion the last run did not.
import type { StoredServer } from "./serverTargets";

export interface KeyValueStore {
  get(key: string): Promise<string | null>;
  set(key: string, value: string): Promise<void>;
  delete(key: string): Promise<void>;
}

const staging = (key: string): string => `${key}.next`;

function parseServers(raw: string | null): StoredServer[] | null {
  if (!raw) return null;
  try {
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return null;
    return parsed.filter(
      (row): row is StoredServer =>
        row != null && typeof row === "object" && typeof (row as StoredServer).id === "string" && typeof (row as StoredServer).baseUrl === "string",
    );
  } catch {
    return null;
  }
}

export async function writeServers(store: KeyValueStore, key: string, servers: readonly StoredServer[]): Promise<void> {
  const json = JSON.stringify(servers);
  await store.set(staging(key), json);
  await store.set(key, json);
  await store.delete(staging(key));
}

export async function readServers(store: KeyValueStore, key: string): Promise<StoredServer[]> {
  const pending = parseServers(await store.get(staging(key)));
  if (pending) {
    await store.set(key, JSON.stringify(pending));
    await store.delete(staging(key));
    return pending;
  }
  return parseServers(await store.get(key)) ?? [];
}
