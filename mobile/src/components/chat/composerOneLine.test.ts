import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { textFitsOneLine } from "./composerSizing";

// A textarea whose layout getters are instrumented: the point of `textFitsOneLine` is that typing
// a character never forces a synchronous layout (collapse to 0px + read scrollHeight did, twice
// per keystroke, over the whole page).
function fakeTextarea(clientWidth: number) {
  const reads = { clientWidth: 0, scrollHeight: 0 };
  const node = {
    get clientWidth() {
      reads.clientWidth += 1;
      return clientWidth;
    },
    get scrollHeight() {
      reads.scrollHeight += 1;
      return 22;
    },
  };
  return { node: node as unknown as HTMLTextAreaElement, reads };
}

describe("textFitsOneLine", () => {
  let resizeCallbacks: ((entries: { contentRect: { width: number } }[]) => void)[] = [];

  beforeEach(() => {
    resizeCallbacks = [];
    vi.stubGlobal("document", {
      createElement: () => ({ getContext: () => ({ font: "", measureText: (text: string) => ({ width: text.length * 8 }) }) }),
    });
    vi.stubGlobal("getComputedStyle", () => ({
      paddingLeft: "0px",
      paddingRight: "0px",
      fontStyle: "normal",
      fontWeight: "400",
      fontSize: "15px",
      fontFamily: "Geist",
      letterSpacing: "normal",
    }));
    vi.stubGlobal(
      "ResizeObserver",
      class {
        constructor(cb: (entries: { contentRect: { width: number } }[]) => void) {
          resizeCallbacks.push(cb);
        }
        observe() {}
      },
    );
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("answers from canvas metrics for short single-line text without reading scrollHeight", () => {
    const { node, reads } = fakeTextarea(400);
    expect(textFitsOneLine(node, "hello world")).toBe(true);
    expect(textFitsOneLine(node, "hello world!")).toBe(true);
    expect(reads.scrollHeight).toBe(0);
  });

  it("reads layout width once per node, then follows the ResizeObserver", () => {
    const { node, reads } = fakeTextarea(400);
    textFitsOneLine(node, "a");
    textFitsOneLine(node, "ab");
    textFitsOneLine(node, "abc");
    expect(reads.clientWidth).toBe(1);

    // 40 chars * 8px = 320px: fits in 400px, but not once the column narrows to 300px.
    const text = "x".repeat(40);
    expect(textFitsOneLine(node, text)).toBe(true);
    resizeCallbacks[0]([{ contentRect: { width: 300 } }]);
    expect(textFitsOneLine(node, text)).toBe(false);
  });

  it("defers to the exact DOM measure for newlines and for text near the line width", () => {
    const { node } = fakeTextarea(400);
    expect(textFitsOneLine(node, "one\ntwo")).toBe(false);
    expect(textFitsOneLine(node, "x".repeat(49))).toBe(false); // 392px > 96% of 400px
    expect(textFitsOneLine(node, "x".repeat(47))).toBe(true); // 376px
  });
});
