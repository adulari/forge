import { describe, expect, it, vi } from "vitest";

import { createStreamStore } from "./streamStore";

describe("createStreamStore", () => {
  it("notifies subscribers once per changed value", () => {
    const store = createStreamStore();
    const listener = vi.fn();
    const off = store.subscribe(listener);
    store.set("a");
    store.set("a");
    store.set("ab");
    expect(listener).toHaveBeenCalledTimes(2);
    expect(store.get()).toBe("ab");
    off();
    store.set("abc");
    expect(listener).toHaveBeenCalledTimes(2);
  });

  it("remembers the newest non-empty text through a cleared reply", () => {
    const store = createStreamStore();
    store.set("hello");
    store.set("");
    expect(store.get()).toBe("");
    expect(store.last()).toBe("hello");
  });

  it("forgets the last turn's text when a new turn starts", () => {
    const store = createStreamStore();
    store.set("old");
    store.set("");
    store.rememberCurrent();
    expect(store.last()).toBe("");
    store.set("new");
    expect(store.last()).toBe("new");
  });
});
