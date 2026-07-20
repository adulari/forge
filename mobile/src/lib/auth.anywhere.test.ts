import { describe, expect, it } from "vitest";

import {
  mergeAnywhereHosts,
  reconcileAnywhereHosts,
  unrepresentedAnywhereHosts,
  type StoredServer,
} from "./serverTargets";

const direct: StoredServer = {
  id: "direct-1",
  name: "Local workstation",
  baseUrl: "https://local.test/token",
  token: "daemon-secret",
  host: "local.test",
  addedAt: 10,
};

describe("Anywhere host target reconciliation", () => {
  it("preserves direct targets exactly while adding managed hosts", () => {
    const result = mergeAnywhereHosts([direct], [{ id: "a".repeat(32), name: "Laptop" }], 20);
    expect(result[0]).toBe(direct);
    expect(result[1]).toEqual({
      id: `anywhere:${"a".repeat(32)}`,
      name: "Laptop",
      baseUrl: `fany://${"a".repeat(32)}`,
      token: "",
      host: "Laptop",
      addedAt: 20,
      transport: "anywhere",
    });
  });

  it("removes only stale managed hosts and retains their original added time", () => {
    const managed = mergeAnywhereHosts([], [{ id: "b".repeat(32), name: "Old name" }], 30)[0];
    const result = mergeAnywhereHosts([direct, managed], [{ id: "b".repeat(32), name: "New name" }], 99);
    expect(result).toHaveLength(2);
    expect(result[0]).toBe(direct);
    expect(result[1]).toMatchObject({ name: "New name", addedAt: 30 });
    expect(mergeAnywhereHosts([direct, managed], [])).toEqual([direct]);
  });

  it("waits for a cold-start direct target read before persisting managed hosts", async () => {
    let finishLoad: ((servers: StoredServer[]) => void) | undefined;
    const load = new Promise<StoredServer[]>((resolve) => { finishLoad = resolve; });
    const saved: StoredServer[][] = [];
    const reconciliation = reconcileAnywhereHosts(
      () => load,
      async (next) => { saved.push(next); },
      [{ id: "c".repeat(32), name: "Remote" }],
    );
    await Promise.resolve();
    expect(saved).toEqual([]);
    finishLoad?.([direct]);
    await expect(reconciliation).resolves.toMatchObject([direct, { transport: "anywhere" }]);
    expect(saved[0]?.[0]).toEqual(direct);
  });

  it("does not render a managed host twice as a server and a Fleet host filter", () => {
    const host = { id: "d".repeat(32), name: "archlinux" };
    const managed = mergeAnywhereHosts([], [host], 40);
    expect(unrepresentedAnywhereHosts(managed, [host])).toEqual([]);
    expect(unrepresentedAnywhereHosts([direct], [host])).toEqual([host]);
  });
});
