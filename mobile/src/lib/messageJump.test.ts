import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  clearMessageJump,
  findJumpIndex,
  peekMessageJump,
  requestMessageJump,
  subscribeMessageJump,
} from "./messageJump";

describe("messageJump", () => {
  beforeEach(() => clearMessageJump("s1"));

  it("holds a request for its own session only", () => {
    requestMessageJump("s1", 42);
    expect(peekMessageJump("s1")).toBe(42);
    expect(peekMessageJump("s2")).toBeNull();
    clearMessageJump("s2");
    expect(peekMessageJump("s1")).toBe(42);
    clearMessageJump("s1");
    expect(peekMessageJump("s1")).toBeNull();
  });

  it("notifies subscribers when a request arrives or is consumed", () => {
    const listener = vi.fn();
    const stop = subscribeMessageJump(listener);
    requestMessageJump("s1", 7);
    clearMessageJump("s1");
    expect(listener).toHaveBeenCalledTimes(2);
    stop();
    requestMessageJump("s1", 8);
    expect(listener).toHaveBeenCalledTimes(2);
  });

  it("finds the row for a seq in the loaded timeline", () => {
    const items = [{ id: "streaming" }, { id: "h90" }, { id: "t88-1" }, { id: "h87" }];
    expect(findJumpIndex(items, 87)).toBe(3);
    expect(findJumpIndex(items, 12)).toBe(-1);
  });
});
