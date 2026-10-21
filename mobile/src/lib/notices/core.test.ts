import { describe, expect, it } from "vitest";

import {
  FRESH_WINDOW_SECS,
  MAX_ITEMS,
  buildNoticeRows,
  bannerCopy,
  emptyHost,
  genericAlertsToDismiss,
  GENERIC_PUSH_BODY,
  isUnseen,
  markSeen,
  mergeNotices,
  osCopy,
  parseNoticesResponse,
  parsePersisted,
  sessionIdOf,
  sessionLabel,
  syncHost,
  unseenCount,
  type Notice,
  type NoticesResponse,
} from "./core";

const NOW = 1_000_000;

function notice(seq: number, over: Partial<Notice> = {}): Notice {
  return { seq, at: NOW - 10 + seq, title: `t${seq}`, message: `m${seq}`, priority: "normal", session_id: "s1", ...over };
}

function response(notices: Notice[], latest?: number): NoticesResponse {
  return { notices, latest: latest ?? Math.max(0, ...notices.map((n) => n.seq)) };
}

describe("parseNoticesResponse", () => {
  it("maps the daemon shape and defaults a blank title", () => {
    const parsed = parseNoticesResponse({
      notices: [{ seq: 2, at: 5, title: " ", message: "hi", priority: "high", session_id: "abc" }],
      latest: 2,
    });
    expect(parsed).toEqual({
      notices: [{ seq: 2, at: 5, title: "Forge", message: "hi", priority: "high", session_id: "abc" }],
      latest: 2,
    });
  });

  it("drops malformed entries instead of failing the whole sync", () => {
    const parsed = parseNoticesResponse({ notices: [null, { seq: "1" }, { seq: 0, at: 1, message: "x" }, { seq: 3, at: 1, message: "ok" }], latest: 3 });
    expect(parsed.notices.map((n) => n.seq)).toEqual([3]);
  });

  it("rejects a body that is not the notices shape", () => {
    expect(() => parseNoticesResponse({ error: "nope" })).toThrow();
    expect(() => parseNoticesResponse(null)).toThrow();
  });

  it("never reports a latest below the highest notice it returned", () => {
    expect(parseNoticesResponse({ notices: [{ seq: 9, at: 1, message: "x" }], latest: 4 }).latest).toBe(9);
  });
});

describe("mergeNotices", () => {
  it("dedupes, orders newest first and advances the cursor", () => {
    const first = mergeNotices(emptyHost(), response([notice(1), notice(2)]), NOW);
    const second = mergeNotices(first.host, response([notice(2), notice(3)]), NOW);
    expect(second.host.items.map((n) => n.seq)).toEqual([3, 2, 1]);
    expect(second.host.cursor).toBe(3);
    expect(second.fresh.map((n) => n.seq)).toEqual([3]);
  });

  it("announces a notice once even across a relaunch (persisted marks only)", () => {
    const first = mergeNotices(emptyHost(), response([notice(1), notice(2)]), NOW);
    const relaunched = emptyHost({ notified: first.host.notified, seen: first.host.seen });
    const again = mergeNotices(relaunched, response([notice(1), notice(2)]), NOW);
    expect(again.fresh).toEqual([]);
    expect(again.host.items).toHaveLength(2);
  });

  it("lists but does not announce notices older than the freshness window", () => {
    const old = notice(1, { at: NOW - FRESH_WINDOW_SECS - 1 });
    const result = mergeNotices(emptyHost(), response([old, notice(2)]), NOW);
    expect(result.fresh.map((n) => n.seq)).toEqual([2]);
    expect(result.host.items).toHaveLength(2);
    expect(result.host.notified.seq).toBe(2);
  });

  it("caps the held list", () => {
    const many = Array.from({ length: MAX_ITEMS + 10 }, (_, i) => notice(i + 1));
    expect(mergeNotices(emptyHost(), response(many), NOW).host.items).toHaveLength(MAX_ITEMS);
  });

  it("announces a post-restart notice whose seq restarted below the old mark", () => {
    const before = mergeNotices(emptyHost(), response([notice(1), notice(2), notice(3)]), NOW);
    const restarted = { ...before.host, items: [], cursor: 0 };
    const after = mergeNotices(restarted, response([notice(1, { at: NOW + 60, title: "new" })]), NOW + 60);
    expect(after.fresh.map((n) => n.title)).toEqual(["new"]);
  });
});

describe("syncHost", () => {
  it("fetches after the cursor", async () => {
    const calls: number[] = [];
    const host = { ...emptyHost(), cursor: 4 };
    await syncHost(host, async (after) => (calls.push(after), response([notice(5)])), NOW);
    expect(calls).toEqual([4]);
  });

  it("re-reads from zero when the daemon's latest falls below the cursor (restart)", async () => {
    const calls: number[] = [];
    const host = { ...mergeNotices(emptyHost(), response([notice(7)]), NOW).host };
    const result = await syncHost(
      host,
      async (after) => {
        calls.push(after);
        return after === 0 ? response([notice(1, { at: NOW + 5, title: "post-restart" })]) : { notices: [], latest: 1 };
      },
      NOW + 5,
    );
    expect(calls).toEqual([7, 0]);
    expect(result.host.items.map((n) => n.title)).toEqual(["post-restart"]);
    expect(result.host.cursor).toBe(1);
    expect(result.fresh).toHaveLength(1);
  });

  it("propagates a fetch failure so the store can retry later", async () => {
    await expect(syncHost(emptyHost(), async () => { throw new Error("offline"); }, NOW)).rejects.toThrow("offline");
  });
});

describe("seen tracking", () => {
  it("counts unseen and marks through a given notice without moving backwards", () => {
    const host = mergeNotices(emptyHost(), response([notice(1), notice(2), notice(3)]), NOW).host;
    expect(unseenCount(host)).toBe(3);
    const partly = markSeen(host, host.items.find((n) => n.seq === 2));
    expect(unseenCount(partly)).toBe(1);
    expect(markSeen(partly, host.items.find((n) => n.seq === 1)).seen).toEqual(partly.seen);
    expect(unseenCount(markSeen(partly))).toBe(0);
    expect(isUnseen(markSeen(partly), notice(4))).toBe(true);
  });
});

describe("presentation", () => {
  it("opens a session only for a well-formed, non-fallback id", () => {
    expect(sessionIdOf(notice(1, { session_id: "abc-123" }))).toBe("abc-123");
    expect(sessionIdOf(notice(1, { session_id: "api" }))).toBeNull();
    expect(sessionIdOf(notice(1, { session_id: "../x" }))).toBeNull();
    expect(sessionIdOf(notice(1, { session_id: "" }))).toBeNull();
  });

  it("labels the session by title, falling back to a short id", () => {
    const n = notice(1, { session_id: "abcdef123456" });
    expect(sessionLabel(n, new Map([["abcdef123456", "Fix mesh"]]))).toBe("Fix mesh");
    expect(sessionLabel(n, new Map())).toBe("session abcdef12");
    expect(sessionLabel(notice(1, { session_id: "api" }), new Map())).toBeNull();
  });

  it("collapses a burst into one banner", () => {
    expect(bannerCopy([notice(1)])).toEqual({ title: "t1", body: "m1" });
    expect(bannerCopy([notice(2), notice(1)])).toEqual({ title: "2 new notices", body: "t2: m2" });
  });

  it("keeps the real text off OS surfaces while the app lock is on", () => {
    expect(osCopy(notice(1), false)).toEqual({ title: "t1", body: "m1" });
    const locked = osCopy(notice(1), true);
    expect(`${locked.title} ${locked.body}`).not.toContain("m1");
  });

  it("builds per-host rows in server order, newest first, with unseen flags", () => {
    const a = markSeen(mergeNotices(emptyHost(), response([notice(1)]), NOW).host);
    const b = mergeNotices(emptyHost(), response([notice(1), notice(2)]), NOW).host;
    const rows = buildNoticeRows(
      [{ id: "a", name: "Laptop" }, { id: "empty", name: "Idle" }, { id: "b", name: "Desktop" }],
      { a, b },
    );
    expect(rows.map((r) => (r.kind === "header" ? r.label : `${r.serverId}${r.notice.seq}${r.unseen ? "*" : ""}`))).toEqual([
      "Laptop", "a1", "Desktop", "b2*", "b1*",
    ]);
  });
});

describe("genericAlertsToDismiss", () => {
  const presented = [
    { id: "push", body: GENERIC_PUSH_BODY, own: false },
    { id: "locked-local", body: GENERIC_PUSH_BODY, own: true },
    { id: "real", body: "m1", own: true },
    { id: "untitled", body: null, own: false },
  ];

  it("keeps generic alerts when no real-text notification replaced them", () => {
    expect(genericAlertsToDismiss(presented, 0)).toEqual([]);
  });

  it("clears only hosted generic alerts once a replacement was raised", () => {
    expect(genericAlertsToDismiss(presented, 1)).toEqual(["push"]);
  });
});

describe("parsePersisted", () => {
  it("round-trips marks and survives junk", () => {
    expect(parsePersisted(null)).toEqual({});
    expect(parsePersisted("{nope")).toEqual({});
    expect(parsePersisted(JSON.stringify({ a: { notified: { seq: 2, at: 9 }, seen: { seq: 1, at: 4 } }, b: 3 }))).toEqual({
      a: { notified: { seq: 2, at: 9 }, seen: { seq: 1, at: 4 } },
    });
  });
});
