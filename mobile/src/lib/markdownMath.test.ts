import { describe, expect, it } from "vitest";

import { INLINE_MATH_SOURCE, parseMathBlock } from "./markdownMath";

const inline = (text: string) => {
  const re = new RegExp(INLINE_MATH_SOURCE, "g");
  return [...text.matchAll(re)].map((m) => m[1] ?? m[2] ?? m[3]);
};

describe("inline math", () => {
  it("matches $x$, $$x$$ and \\(x\\)", () => {
    expect(inline("so $E=mc^2$ holds")).toEqual(["E=mc^2"]);
    expect(inline("so $$a+b$$ holds")).toEqual(["a+b"]);
    expect(inline(String.raw`so \(a_i\) holds`)).toEqual(["a_i"]);
  });

  it("leaves prices alone", () => {
    expect(inline("it costs $5 and then $10 more")).toEqual([]);
    expect(inline("$ 5 $")).toEqual([]);
    expect(inline("pay $5, get $x$ back")).toEqual(["x"]);
  });

  it("does not open inside a word or after a backslash", () => {
    expect(inline("a$b$ and \\$c$")).toEqual([]);
  });
});

describe("parseMathBlock", () => {
  it("reads a one-line block", () => {
    expect(parseMathBlock(["$$ x^2 $$", "after"], 0)).toEqual({ tex: "x^2", next: 1 });
    expect(parseMathBlock([String.raw`\[ y \]`], 0)).toEqual({ tex: "y", next: 1 });
  });

  it("reads a multi-line block", () => {
    const lines = ["$$", "a = b", "c = d", "$$", "text"];
    expect(parseMathBlock(lines, 0)).toEqual({ tex: "a = b\nc = d", next: 4 });
  });

  it("stays text until the block closes", () => {
    expect(parseMathBlock(["$$", "a = b"], 0)).toBeNull();
  });

  it("is not triggered by text after the close or by ordinary lines", () => {
    expect(parseMathBlock(["$$ x $$ and more"], 0)).toBeNull();
    expect(parseMathBlock(["plain"], 0)).toBeNull();
  });
});
