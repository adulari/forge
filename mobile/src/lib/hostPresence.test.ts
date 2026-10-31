import { describe, expect, it } from "vitest";

import {
  canActivate,
  firstEnabledId,
  seenUpdates,
  EMPTY_PRESENCE,
  enabledServers,
  hostBadge,
  lastSeenLabel,
  pickWarmServer,
  pruneRemoved,
  recordSeen,
  recordSelected,
  setServerDisabled,
} from "./hostPresence";
import type { StoredServer } from "./serverTargets";

const server = (id: string, extra: Partial<StoredServer> = {}): StoredServer => ({
  id,
  name: id,
  baseUrl: `https://${id}`,
  token: id,
  host: id,
  addedAt: 1,
  ...extra,
});

describe("pickWarmServer", () => {
  const servers = [server("a"), server("b"), server("c", { disabled: true })];
  it("is the most recent other enabled host", () => {
    expect(pickWarmServer(servers, "a", ["a", "c", "b"])?.id).toBe("b");
  });
  it("is none when nothing else was ever selected", () => {
    expect(pickWarmServer(servers, "a", ["a"])).toBeNull();
    expect(pickWarmServer(servers, "a", [])).toBeNull();
  });
});

describe("hostBadge", () => {
  const base = { id: "b", activeId: "a", warmId: "b", disabled: false, reachable: true };
  it("names each state", () => {
    expect(hostBadge({ ...base, id: "a" })).toBe("selected");
    expect(hostBadge(base)).toBe("kept-ready");
    expect(hostBadge({ ...base, warmId: null })).toBe("connected");
    expect(hostBadge({ ...base, reachable: false })).toBe("offline");
    expect(hostBadge({ ...base, disabled: true })).toBe("disabled");
  });
});

describe("lastSeenLabel", () => {
  it("reads in the largest sensible unit", () => {
    const now = 10_000_000;
    expect(lastSeenLabel(undefined, now)).toBe("not seen yet");
    expect(lastSeenLabel(now - 5_000, now)).toBe("last seen just now");
    expect(lastSeenLabel(now - 4 * 60_000, now)).toBe("last seen 4m ago");
    expect(lastSeenLabel(now - 3 * 3_600_000, now)).toBe("last seen 3h ago");
    expect(lastSeenLabel(now - 2 * 86_400_000, now)).toBe("last seen 2d ago");
  });
});

describe("presence updates", () => {
  it("keeps recency order without duplicates", () => {
    let s = recordSelected(EMPTY_PRESENCE, "a");
    s = recordSelected(s, "b");
    s = recordSelected(s, "a");
    expect(s.recent).toEqual(["a", "b"]);
    expect(recordSelected(s, "a")).toBe(s);
  });
  it("throttles last-seen writes to once a minute", () => {
    const s = recordSeen(EMPTY_PRESENCE, "a", 1_000_000);
    expect(recordSeen(s, "a", 1_030_000)).toBe(s);
    expect(recordSeen(s, "a", 1_070_000).lastSeen.a).toBe(1_070_000);
  });
  it("forgets removed servers", () => {
    const s = recordSeen(recordSelected(EMPTY_PRESENCE, "gone"), "gone", 1);
    expect(pruneRemoved(s, [server("a")])).toEqual({ lastSeen: {}, recent: [] });
  });
});

describe("setServerDisabled", () => {
  it("toggles only the named server and leaves the rest byte-for-byte", () => {
    const list = [server("a"), server("b")];
    const off = setServerDisabled(list, "b", true);
    expect(off[0]).toBe(list[0]);
    expect(off[1].disabled).toBe(true);
    expect(enabledServers(off).map((s) => s.id)).toEqual(["a"]);
    const on = setServerDisabled(off, "b", false);
    expect("disabled" in on[1]).toBe(false);
  });
});

describe("seenUpdates", () => {
  const servers = [{ id: "a" }, { id: "b" }];
  it("changes with every successful poll, not only when the reachable set changes", () => {
    const first = seenUpdates(servers, [{ isSuccess: true, dataUpdatedAt: 100 }, { isSuccess: false }]);
    const later = seenUpdates(servers, [{ isSuccess: true, dataUpdatedAt: 160_000 }, { isSuccess: false }]);
    expect(first).toEqual([{ id: "a", at: 100 }]);
    expect(later).toEqual([{ id: "a", at: 160_000 }]);
    expect(JSON.stringify(first)).not.toBe(JSON.stringify(later));
  });
});

describe("disabled servers cannot become active", () => {
  const list = [{ id: "a", disabled: true }, { id: "b" }];
  it("blocks activation and skips them as a fallback", () => {
    expect(canActivate(list, "a")).toBe(false);
    expect(canActivate(list, "b")).toBe(true);
    expect(canActivate(list, "zzz")).toBe(false);
    expect(firstEnabledId(list)).toBe("b");
    expect(firstEnabledId([{ id: "a", disabled: true }])).toBeNull();
  });
});
