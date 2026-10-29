import { describe, expect, it } from "vitest";

import { markTerminalClosed, tabsForSession, terminalsToAdopt, wasTerminalClosedRecently } from "./terminalTabs";

const tab = (sessionId: string | null, resourceId: string | null, kind = "terminal") => ({
  kind,
  sessionId,
  resourceId,
});
const live = (id: string) => ({ terminal_id: id, status: "running" as const });

describe("terminal tabs", () => {
  it("adopts a daemon terminal that has no tab, counting the default tab as the first terminal", () => {
    const tabs = [tab(null, null)];
    expect(terminalsToAdopt(tabs, "s1", [live("term-1"), live("term-2"), live("term-3")])).toEqual(["term-2", "term-3"]);
  });

  it("does not adopt a clean exit, but keeps a failed one to read", () => {
    const daemon = [
      { terminal_id: "term-2", status: "exited" as const, exit_code: 0 },
      { terminal_id: "term-3", status: "exited" as const, exit_code: 3 },
    ];
    expect(terminalsToAdopt([tab(null, null)], "s1", daemon)).toEqual(["term-3"]);
  });

  it("ignores another session's tabs and a tab closed a moment ago", () => {
    markTerminalClosed("s1", "term-2", 1_000);
    expect(wasTerminalClosedRecently("s1", "term-2", 2_000)).toBe(true);
    expect(wasTerminalClosedRecently("s2", "term-2", 2_000)).toBe(false);
    expect(wasTerminalClosedRecently("s1", "term-2", 17_000)).toBe(false);
    markTerminalClosed("s1", "term-2", 1_000);
    expect(terminalsToAdopt([tab("other", "term-4")], "s1", [live("term-2"), live("term-4")], 2_000)).toEqual(["term-4"]);
  });

  it("shows only the focused session's terminals beside the default tab", () => {
    const tabs = [tab(null, null), tab("s1", "term-2"), tab("s2", "term-2"), tab("s1", "a.ts", "files")];
    expect(tabsForSession(tabs, "s1")).toEqual([tabs[0], tabs[1], tabs[3]]);
    expect(tabsForSession(tabs, null)).toEqual([tabs[0], tabs[3]]);
  });
});
