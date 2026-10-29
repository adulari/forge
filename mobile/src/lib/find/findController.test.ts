import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { type FindTarget, findController, resolveFindTarget } from "./findController";

function target(id: FindTarget["id"], hits = 3) {
  let index = -1;
  return {
    id,
    label: id,
    search: vi.fn((query: string) => {
      index = query === "none" ? -1 : 0;
      return { count: query === "none" ? 0 : hits, index };
    }),
    step: vi.fn((direction: 1 | -1) => {
      index = (index + direction + hits) % hits;
      return { count: hits, index };
    }),
    clear: vi.fn(),
  };
}

describe("resolveFindTarget", () => {
  it("prefers the focused surface when it can be searched", () => {
    expect(resolveFindTarget(["chat", "diff"], "diff")).toBe("diff");
  });

  it("falls back through the fixed order, and to nothing", () => {
    expect(resolveFindTarget(["editor", "diff"], "chat")).toBe("diff");
    expect(resolveFindTarget(["terminal"], null)).toBe("terminal");
    expect(resolveFindTarget([], "chat")).toBeNull();
  });
});

describe("findController", () => {
  const cleanups: (() => void)[] = [];
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => {
    findController.close();
    cleanups.splice(0).forEach((fn) => fn());
    vi.useRealTimers();
  });

  it("claims nothing while no surface is registered", () => {
    expect(findController.request()).toBe(false);
    expect(findController.getState().open).toBe(false);
  });

  it("opens on the focused target and searches after the debounce", async () => {
    const chat = target("chat");
    const diff = target("diff");
    cleanups.push(findController.register(chat), findController.register(diff));
    findController.markFocus("diff");
    expect(findController.request()).toBe(true);
    expect(findController.getState().targetId).toBe("diff");
    findController.setQuery("foo");
    expect(diff.search).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(200);
    expect(diff.search).toHaveBeenCalledWith("foo", false);
    expect(findController.getState().result).toEqual({ count: 3, index: 0 });
  });

  it("steps through hits and wraps", async () => {
    cleanups.push(findController.register(target("chat")));
    findController.request();
    findController.setQuery("foo");
    await vi.advanceTimersByTimeAsync(200);
    await findController.step(-1);
    expect(findController.getState().result.index).toBe(2);
    await findController.step(1);
    expect(findController.getState().result.index).toBe(0);
  });

  it("ignores a search that was superseded while it ran", async () => {
    let release: (r: { count: number; index: number }) => void = () => {};
    const slow: FindTarget = {
      id: "chat",
      label: "chat",
      search: (query) =>
        query === "slow"
          ? new Promise((resolve) => {
              release = resolve;
            })
          : { count: 1, index: 0 },
      step: () => ({ count: 1, index: 0 }),
      clear: () => {},
    };
    cleanups.push(findController.register(slow));
    findController.request();
    findController.setQuery("slow");
    await vi.advanceTimersByTimeAsync(200);
    findController.setQuery("fast");
    await vi.advanceTimersByTimeAsync(200);
    release({ count: 99, index: 7 });
    await vi.advanceTimersByTimeAsync(0);
    expect(findController.getState().result).toEqual({ count: 1, index: 0 });
  });

  it("clears highlights on close and closes when its target goes away", () => {
    const chat = target("chat");
    const off = findController.register(chat);
    findController.request();
    findController.close();
    expect(chat.clear).toHaveBeenCalled();
    findController.request();
    off();
    expect(findController.getState().open).toBe(false);
    expect(findController.hasTargets()).toBe(false);
  });
});
