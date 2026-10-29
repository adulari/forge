import { describe, expect, it } from "vitest";

import { splitStreamedText } from "./streamCap";

const para = (n: number) => `paragraph ${n} ${"x".repeat(80)}`;

describe("splitStreamedText", () => {
  it("leaves a short reply and a settled reply whole", () => {
    expect(splitStreamedText("short", true, 100)).toEqual({ head: "", tail: "short" });
    const long = Array.from({ length: 40 }, (_, i) => para(i)).join("\n\n");
    expect(splitStreamedText(long, false, 100)).toEqual({ head: "", tail: long });
  });

  it("splits a long streaming reply on a paragraph break, newest part last", () => {
    const long = Array.from({ length: 40 }, (_, i) => para(i)).join("\n\n");
    const { head, tail } = splitStreamedText(long, true, 1000);
    expect(head.length).toBeGreaterThan(0);
    expect(`${head}\n\n${tail}`).toBe(long);
    expect(tail.length).toBeLessThanOrEqual(1000);
    expect(tail.startsWith("paragraph")).toBe(true);
  });

  it("never cuts inside a code fence", () => {
    const code = "```ts\n" + Array.from({ length: 60 }, (_, i) => `const a${i} = ${i};\n\nconst b${i} = 1;`).join("\n\n") + "\n```";
    const text = `${para(0)}\n\n${code}`;
    const { head, tail } = splitStreamedText(text, true, 400);
    expect((head.match(/```/g) ?? []).length % 2).toBe(0);
    expect(`${head}${head ? "\n\n" : ""}${tail}`).toBe(text);
  });
});
