import { describe, expect, it } from "vitest";

import { type KeyValueStore, readServers, writeServers } from "./serverStoreJournal";
import type { StoredServer } from "./serverTargets";

const server = (id: string): StoredServer => ({ id, name: id, baseUrl: `http://${id}`, token: "t", host: id, addedAt: 1 });

const memory = (seed: Record<string, string> = {}): KeyValueStore & { data: Record<string, string> } => {
  const data = { ...seed };
  return {
    data,
    get: async (key) => data[key] ?? null,
    set: async (key, value) => {
      data[key] = value;
    },
    delete: async (key) => {
      delete data[key];
    },
  };
};

describe("server list journal", () => {
  it("round-trips and leaves no staging key behind", async () => {
    const store = memory();
    await writeServers(store, "servers", [server("a"), server("b")]);
    expect(Object.keys(store.data)).toEqual(["servers"]);
    expect((await readServers(store, "servers")).map((s) => s.id)).toEqual(["a", "b"]);
  });

  it("finishes a write that died after staging", async () => {
    const store = memory({ servers: JSON.stringify([server("old")]), "servers.next": JSON.stringify([server("old"), server("new")]) });
    expect((await readServers(store, "servers")).map((s) => s.id)).toEqual(["old", "new"]);
    expect(JSON.parse(store.data.servers).map((s: StoredServer) => s.id)).toEqual(["old", "new"]);
    expect(store.data["servers.next"]).toBeUndefined();
  });

  it("ignores a half-written staging value and keeps the last good list", async () => {
    const store = memory({ servers: JSON.stringify([server("old")]), "servers.next": '[{"id":"ne' });
    expect((await readServers(store, "servers")).map((s) => s.id)).toEqual(["old"]);
  });

  it("drops malformed rows instead of throwing", async () => {
    const store = memory({ servers: JSON.stringify([server("a"), null, { id: 3 }, { id: "x" }]) });
    expect((await readServers(store, "servers")).map((s) => s.id)).toEqual(["a"]);
  });

  it("reads an empty or corrupt store as no servers", async () => {
    expect(await readServers(memory(), "servers")).toEqual([]);
    expect(await readServers(memory({ servers: "{oops" }), "servers")).toEqual([]);
  });
});
