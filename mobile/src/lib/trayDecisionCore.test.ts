import { describe, expect, it, vi } from "vitest";

import { answerTrayDecision, buildTrayDecision, newestWaiting, trayRowOrder } from "./trayDecisionCore";

const permission = { session_id: "s-new", prompt_seq: 9, permission_prompt: { tool: "shell" }, question: null };

function deps(overrides: Partial<Parameters<typeof answerTrayDecision>[2]> = {}) {
  return {
    appLocked: false,
    currentPromptSeq: () => 9,
    resolveBaseUrl: (id: string) => (id === "srv-b" ? "http://b:7420/tok" : null),
    answer: vi.fn(async () => ({ ok: true })),
    ...overrides,
  };
}

describe("newestWaiting", () => {
  it("picks the most recently active waiting session", () => {
    const rows = [
      { id: "old", waiting: true, last_activity: 10 },
      { id: "idle", waiting: false, last_activity: 99 },
      { id: "s-new", waiting: true, last_activity: 20 },
    ];
    expect(newestWaiting(rows)?.id).toBe("s-new");
    expect(newestWaiting([{ id: "idle", waiting: false, last_activity: 1 }])).toBeNull();
  });
});

describe("buildTrayDecision", () => {
  const base = { title: "Fix calc", serverId: "srv-b", sessionId: "s-new", contentLocked: false };

  it("offers Allow/Deny (a promptSeq) only for a readable permission prompt", () => {
    expect(buildTrayDecision({ ...base, snapshot: permission })).toEqual({ title: "Fix calc", serverId: "srv-b", sessionId: "s-new", promptSeq: 9 });
    expect(buildTrayDecision({ ...base, snapshot: { ...permission, permission_prompt: null, question: "Which db?" } })?.promptSeq).toBeUndefined();
    expect(buildTrayDecision({ ...base, snapshot: permission, contentLocked: true })?.promptSeq).toBeUndefined();
    expect(buildTrayDecision({ ...base, snapshot: null })?.promptSeq).toBeUndefined();
  });

  it("never borrows another session's snapshot", () => {
    expect(buildTrayDecision({ ...base, snapshot: { ...permission, session_id: "other" } })?.promptSeq).toBeUndefined();
  });

  it("has no decision without a server to answer on", () => {
    expect(buildTrayDecision({ ...base, serverId: null, snapshot: permission })).toBeUndefined();
  });
});

describe("answerTrayDecision", () => {
  const target = { serverId: "srv-b", sessionId: "s-new", promptSeq: 9 };

  it("answers the clicked session on the server stored in the click", async () => {
    const d = deps();
    await expect(answerTrayDecision(target, false, d)).resolves.toBe("answered");
    expect(d.answer).toHaveBeenCalledWith("http://b:7420/tok", { session: "s-new", seq: 9, allow: false });
  });

  it("ignores a click whose prompt has since been answered or replaced", async () => {
    const replaced = deps({ currentPromptSeq: () => 10 });
    await expect(answerTrayDecision(target, true, replaced)).resolves.toBe("stale");
    const gone = deps({ currentPromptSeq: () => null });
    await expect(answerTrayDecision(target, true, gone)).resolves.toBe("stale");
    expect(replaced.answer).not.toHaveBeenCalled();
    expect(gone.answer).not.toHaveBeenCalled();
  });

  it("treats a row from a replaced menu, or one without a prompt seq, as stale", async () => {
    const d = deps();
    await expect(answerTrayDecision(null, true, d)).resolves.toBe("stale");
    await expect(answerTrayDecision({ ...target, promptSeq: null }, true, d)).resolves.toBe("stale");
    expect(d.answer).not.toHaveBeenCalled();
  });

  it("never answers while the app is locked", async () => {
    const d = deps({ appLocked: true });
    await expect(answerTrayDecision(target, true, d)).resolves.toBe("locked");
    expect(d.answer).not.toHaveBeenCalled();
  });

  it("reports a removed server and a rejected answer", async () => {
    await expect(answerTrayDecision({ ...target, serverId: "gone" }, true, deps())).resolves.toBe("no-server");
    const rejected = deps({ answer: vi.fn(async () => Promise.reject(new Error("409"))) });
    await expect(answerTrayDecision(target, true, rejected)).resolves.toBe("failed");
  });
});

describe("trayRowOrder", () => {
  const row = (id: string, state: "waiting" | "busy" | "idle", last_activity: number) => ({
    id,
    waiting: state === "waiting",
    busy: state === "busy",
    last_activity,
  });

  it("puts waiting, then busy, then the most recently touched idle threads", () => {
    const ordered = trayRowOrder([
      row("old-idle", "idle", 10),
      row("busy", "busy", 5),
      row("new-idle", "idle", 90),
      row("waiting", "waiting", 1),
    ]);
    expect(ordered.map((item) => item.id)).toEqual(["waiting", "busy", "new-idle", "old-idle"]);
  });

  it("is stable for equal activity and does not mutate its input", () => {
    const input = [row("a", "idle", 1), row("b", "idle", 1)];
    expect(trayRowOrder(input).map((item) => item.id)).toEqual(["a", "b"]);
    expect(input.map((item) => item.id)).toEqual(["a", "b"]);
  });
});
