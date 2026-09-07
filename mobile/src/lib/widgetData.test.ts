import { describe, expect, it, vi } from "vitest";

import type { SessionRow } from "./api";
import { syncWidgetSessions } from "./widgetData";

const { setMock, reloadMock } = vi.hoisted(() => ({
  setMock: vi.fn(),
  reloadMock: vi.fn(),
}));

vi.mock("@bacons/apple-targets", () => ({
  ExtensionStorage: class {
    static reloadWidget = reloadMock;
    set(...args: unknown[]) {
      setMock(...args);
    }
  },
}));

vi.mock("./platform", () => ({ isIOS: true }));

function session(overrides: Partial<SessionRow> = {}): SessionRow {
  return {
    id: "s1",
    title: "Session",
    cwd: "/repo",
    worktree: null,
    busy: false,
    waiting: false,
    cost_usd: 0,
    context_tokens: 0,
    context_limit: null,
    model: "test-model",
    permission_mode: null,
    created_at: 0,
    last_activity: 0,
    ...overrides,
  };
}

describe("syncWidgetSessions", () => {
  it("dedupes an identical top-4 snapshot but writes again once it changes", () => {
    const sessions = [session({ id: "a" }), session({ id: "b", waiting: true })];

    syncWidgetSessions(sessions);
    expect(setMock).toHaveBeenCalledTimes(1);
    expect(reloadMock).toHaveBeenCalledTimes(1);

    // The daemon's fleet-invalidation socket fires this on every refetch (up to 2x/s while any
    // session streams) even when nothing changed — a fresh array with identical content must be
    // a no-op, not another app-group write + WidgetCenter reload.
    syncWidgetSessions([...sessions]);
    expect(setMock).toHaveBeenCalledTimes(1);
    expect(reloadMock).toHaveBeenCalledTimes(1);

    // A real content change (e.g. cost ticking up) must still sync.
    syncWidgetSessions([session({ id: "a", cost_usd: 1.23 }), sessions[1]]);
    expect(setMock).toHaveBeenCalledTimes(2);
    expect(reloadMock).toHaveBeenCalledTimes(2);
  });
});
