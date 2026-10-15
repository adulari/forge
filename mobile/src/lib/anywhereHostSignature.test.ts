import { describe, expect, it } from "vitest";

import { hostIdsKey, hostSyncKey, keepIfSameHosts } from "./anywhereHostSignature";
import type { AnywhereHost } from "./anywhereApi";

const host = (id: string, name = id, heartbeat: string | null = null): AnywhereHost => ({
  id, device_id: `dev-${id}`, name, created_at: "2026-01-01T00:00:00Z", last_heartbeat_at: heartbeat,
} as AnywhereHost);

describe("host signatures", () => {
  it("id key ignores order, identity, names and heartbeats", () => {
    expect(hostIdsKey([host("b"), host("a")])).toBe(hostIdsKey([host("a", "renamed", "x"), host("b")]));
    expect(hostIdsKey([host("a")])).not.toBe(hostIdsKey([host("a"), host("b")]));
  });

  it("sync key changes on rename but not on order", () => {
    expect(hostSyncKey([host("b"), host("a")])).toBe(hostSyncKey([host("a"), host("b")]));
    expect(hostSyncKey([host("a")])).not.toBe(hostSyncKey([host("a", "new name")]));
  });

  it("keeps the previous array for an identical refresh result", () => {
    const previous = [host("a"), host("b")];
    expect(keepIfSameHosts(previous, [host("b"), host("a")])).toBe(previous);
  });

  it("adopts the new array when the set, a name or a heartbeat changed", () => {
    const previous = [host("a")];
    const added = [host("a"), host("b")];
    expect(keepIfSameHosts(previous, added)).toBe(added);
    const renamed = [host("a", "x")];
    expect(keepIfSameHosts(previous, renamed)).toBe(renamed);
    const beat = [host("a", "a", "t")];
    expect(keepIfSameHosts(previous, beat)).toBe(beat);
  });
});
