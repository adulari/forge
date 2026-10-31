import { describe, expect, it } from "vitest";

import {
  consumeComposerFocus,
  getRenderedPanes,
  getVisibleSessionIds,
  onModelPickerRequest,
  requestComposerFocus,
  requestModelPicker,
  setRenderedPanes,
  setVisibleSessionIds,
  subscribeRenderedPanes,
} from "./alertTargets";

describe("alert targets", () => {
  it("hands the composer focus request to the matching session exactly once", () => {
    requestComposerFocus("s1");
    expect(consumeComposerFocus("other")).toBe(false);
    expect(consumeComposerFocus("s1")).toBe(true);
    expect(consumeComposerFocus("s1")).toBe(false);
  });

  it("remembers the visible sessions", () => {
    setVisibleSessionIds(["a", "b"]);
    expect(getVisibleSessionIds()).toEqual(["a", "b"]);
  });

  it("notifies rendered-pane subscribers only when the value changes", () => {
    let calls = 0;
    const stop = subscribeRenderedPanes(() => (calls += 1));
    setRenderedPanes({ count: 2, maximized: null });
    setRenderedPanes({ count: 2, maximized: null });
    expect(calls).toBe(1);
    setRenderedPanes({ count: 1, maximized: null });
    expect(calls).toBe(2);
    setRenderedPanes(null);
    expect(getRenderedPanes()).toBeNull();
    setRenderedPanes(null);
    expect(calls).toBe(3);
    stop();
  });

  it("delivers a model picker request to listeners", () => {
    const seen: string[] = [];
    const stop = onModelPickerRequest((id) => seen.push(id));
    requestModelPicker("s9");
    stop();
    requestModelPicker("s10");
    expect(seen).toEqual(["s9"]);
  });
});
