import { describe, expect, it } from "vitest";

import {
  alertCopy,
  detectFleetAlerts,
  visibleSessionIds,
  type FleetRowState,
  BODY_MAX,
  DEFAULT_NOTIFY_PREFS,
  FOREGROUND_DEDUPE_MS,
  TITLE_MAX,
  categoryForPriority,
  channelForPriority,
  clipNotification,
  isDuplicateAlert,
  parseNotifyPrefs,
  pwaPromptDue,
  rememberAlert,
  routeAlert,
  sanitizeNotifyText,
  type AlertContext,
} from "./notifyPrefsCore";

const ctx = (patch: Partial<AlertContext> = {}): AlertContext => ({
  category: "done",
  prefs: DEFAULT_NOTIFY_PREFS,
  sessionId: "s1",
  visibleSessionIds: ["main"],
  backgrounded: false,
  ...patch,
});

describe("notify prefs parsing", () => {
  it("defaults to every category on, all sessions, silent and with text", () => {
    expect(parseNotifyPrefs(null)).toEqual(DEFAULT_NOTIFY_PREFS);
    expect(parseNotifyPrefs("garbage")).toEqual(DEFAULT_NOTIFY_PREFS);
  });

  it("keeps explicit values and ignores wrong types", () => {
    const prefs = parseNotifyPrefs('{"done":false,"error":"no","filter":"unfocused","sound":true,"hideText":true}');
    expect(prefs).toEqual({ ...DEFAULT_NOTIFY_PREFS, done: false, filter: "unfocused", sound: true, hideText: true });
  });
});

describe("alert routing", () => {
  it("is silent for a disabled category", () => {
    const prefs = { ...DEFAULT_NOTIFY_PREFS, done: false };
    expect(routeAlert(ctx({ prefs }))).toEqual({ toast: false, os: false });
    expect(routeAlert(ctx({ prefs, backgrounded: true }))).toEqual({ toast: false, os: false });
    expect(routeAlert(ctx({ prefs, category: "error" }))).toEqual({ toast: true, os: false });
  });

  it("toasts in front and notifies the OS in the background", () => {
    expect(routeAlert(ctx())).toEqual({ toast: true, os: false });
    expect(routeAlert(ctx({ backgrounded: true }))).toEqual({ toast: false, os: true });
  });

  it("skips toasts for any session visible in a pane but not OS notifications", () => {
    expect(routeAlert(ctx({ sessionId: "main" }))).toEqual({ toast: false, os: false });
    expect(routeAlert(ctx({ sessionId: "main", backgrounded: true }))).toEqual({ toast: false, os: true });
    expect(routeAlert(ctx({ sessionId: "right", visibleSessionIds: ["main", "right"] }))).toEqual({ toast: false, os: false });
  });

  it("the unfocused filter drops in-app toasts", () => {
    const prefs = { ...DEFAULT_NOTIFY_PREFS, filter: "unfocused" as const };
    expect(routeAlert(ctx({ prefs }))).toEqual({ toast: false, os: false });
    expect(routeAlert(ctx({ prefs, backgrounded: true }))).toEqual({ toast: false, os: true });
  });
});

describe("notification text", () => {
  it("strips escape sequences and control characters", () => {
    expect(sanitizeNotifyText("\u001b[31mred\u001b[0m \u001b]0;title\u0007ok\u0000\tx\ny", 80)).toBe("red ok x y");
  });

  it("truncates titles to 80 and bodies to 240 by code point", () => {
    const { title, body } = clipNotification("t".repeat(200), "😀".repeat(500));
    expect(Array.from(title)).toHaveLength(TITLE_MAX);
    expect(title.endsWith("…")).toBe(true);
    expect(Array.from(body ?? "")).toHaveLength(BODY_MAX);
    expect(clipNotification("", "b").title).toBe("Forge");
    expect(clipNotification("a").body).toBeUndefined();
  });
});

describe("foreground dedupe", () => {
  it("drops a repeat inside the window and allows it after", () => {
    const seen = new Map<string, number>();
    expect(isDuplicateAlert(seen, "k", 1000)).toBe(false);
    rememberAlert(seen, "k", 1000);
    expect(isDuplicateAlert(seen, "k", 1000 + FOREGROUND_DEDUPE_MS - 1)).toBe(true);
    expect(isDuplicateAlert(seen, "k", 1000 + FOREGROUND_DEDUPE_MS)).toBe(false);
    expect(isDuplicateAlert(seen, "other", 1001)).toBe(false);
  });
});

describe("android channels", () => {
  it("sends high priority notices to attention and the rest to status", () => {
    expect(channelForPriority("high")).toBe("attention");
    expect(channelForPriority("normal")).toBe("status");
    expect(categoryForPriority("high")).toBe("attention");
    expect(categoryForPriority("normal")).toBe("done");
  });
});

describe("installed iOS web app prompt", () => {
  const base = { isIOS: true, standalone: true, permission: "default" as const, dismissed: false };
  it("shows once, only on an installed iOS web app with the answer still open", () => {
    expect(pwaPromptDue(base)).toBe(true);
    expect(pwaPromptDue({ ...base, dismissed: true })).toBe(false);
    expect(pwaPromptDue({ ...base, permission: "granted" })).toBe(false);
    expect(pwaPromptDue({ ...base, standalone: false })).toBe(false);
    expect(pwaPromptDue({ ...base, isIOS: false })).toBe(false);
  });
});

const row = (id: string, patch: Partial<FleetRowState> = {}): FleetRowState => ({ id, title: id, busy: false, waiting: false, ...patch });

describe("fleet alerts", () => {
  it("reports a session that started waiting, finished or failed", () => {
    const before = new Map([
      ["a", row("a", { busy: true })],
      ["b", row("b", { busy: true })],
      ["c", row("c")],
      ["d", row("d", { busy: true })],
    ]);
    const alerts = detectFleetAlerts(before, [
      row("a"),
      row("b", { busy: false, last_turn_outcome: "failed" }),
      row("c", { waiting: true }),
      row("d", { waiting: true }),
      row("new", { waiting: true }),
    ]);
    expect(alerts.map((a) => [a.sessionId, a.category])).toEqual([
      ["a", "done"],
      ["b", "error"],
      ["c", "attention"],
      ["d", "attention"],
    ]);
  });

  it("stays quiet when nothing changed", () => {
    const rows = [row("a", { busy: true }), row("b", { waiting: true })];
    expect(detectFleetAlerts(new Map(rows.map((r) => [r.id, r])), rows)).toEqual([]);
  });
});

describe("visible sessions", () => {
  it("counts every rendered pane and only the maximized one when set", () => {
    expect(visibleSessionIds(["main", "right", null], null, 3)).toEqual(["main", "right"]);
    expect(visibleSessionIds(["main", "right"], 1, 2)).toEqual(["right"]);
    expect(visibleSessionIds([null], null, 1)).toEqual([]);
  });

  it("counts only the routed session when no pane grid is mounted (compact or narrow windows)", () => {
    expect(visibleSessionIds(["main", "right", "third"], null, null)).toEqual(["main"]);
  });

  it("does not count panes the grid hides on a narrow window", () => {
    expect(visibleSessionIds(["a", "b", "c", "d"], null, 2)).toEqual(["a", "b"]);
    expect(visibleSessionIds(["a", "b", "c", "d"], null, 1)).toEqual(["a"]);
  });

  it("a maximized pane overrides the rendered count", () => {
    expect(visibleSessionIds(["a", "b", "c"], 2, 1)).toEqual(["c"]);
  });
});

describe("alert copy", () => {
  it("names the session, or says nothing when text is hidden", () => {
    const alert = { sessionId: "a", title: "Fix login", category: "attention" as const };
    expect(alertCopy(alert, false)).toEqual({ toast: "Fix login needs you", title: "Fix login", body: "This session needs you." });
    expect(alertCopy(alert, true)).toEqual({ toast: "Fix login needs you", title: "Forge", body: "A session needs you." });
    expect(alertCopy({ ...alert, category: "done" }, true).body).toBe("A session finished.");
  });
});
