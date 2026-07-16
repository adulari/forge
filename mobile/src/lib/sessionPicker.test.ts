import { describe, expect, it } from "vitest";

import type { SessionRow } from "./api";
import { filterSessions, isOfflineError, sessionPickerState } from "./sessionPicker";

const row = (id: string, title: string, state: "waiting" | "busy" | "idle"): SessionRow => ({
  id, title, cwd: `/work/${id}`, worktree: null, busy: state === "busy", waiting: state === "waiting",
  cost_usd: 0, context_tokens: 0, context_limit: null, model: "mock", created_at: 0, last_activity: 0,
});

describe("session picker", () => {
  const rows = [row("a", "Forge", "waiting"), row("b", "Docs", "busy")];

  it("filters title, path, status, and waiting-only without mutating server order", () => {
    expect(filterSessions(rows, "docs").map((item) => item.id)).toEqual(["b"]);
    expect(filterSessions(rows, "waiting", true).map((item) => item.id)).toEqual(["a"]);
    expect(filterSessions(rows, "", false)).toEqual(rows);
  });

  it("has explicit states for loading, failure, empty, and ready", () => {
    expect(sessionPickerState({ isLoading: true, isError: false, visibleCount: 0 })).toBe("loading");
    expect(sessionPickerState({ isLoading: false, isError: true, visibleCount: 0 })).toBe("error");
    expect(sessionPickerState({ isLoading: false, isError: false, visibleCount: 0 })).toBe("empty");
    expect(sessionPickerState({ isLoading: false, isError: false, visibleCount: 1 })).toBe("ready");
  });

  it("recognizes network failures for a non-color offline message", () => {
    expect(isOfflineError(new TypeError("Failed to fetch"))).toBe(true);
    expect(isOfflineError(new Error("invalid cwd"))).toBe(false);
  });
});
