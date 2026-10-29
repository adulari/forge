import { describe, expect, it } from "vitest";

import { parseBlocks, parseInlineNodes } from "./markdownParse";

const blocks = (text: string, mentions = false) => parseBlocks(text, mentions);

describe("blocks", () => {
  it("keeps GFM tables", () => {
    const [table] = blocks("| a | b |\n|---|---|\n| 1 | 2 |");
    expect(table.kind).toBe("table");
  });

  it("reads display math between $$ lines and in one line", () => {
    expect(blocks("before\n\n$$\nE = mc^2\n$$\n\nafter").map((b) => b.kind)).toEqual(["paragraph", "math", "paragraph"]);
    expect(blocks("$$a^2+b^2$$")[0]).toMatchObject({ kind: "math", tex: "a^2+b^2" });
  });

  it("leaves an unclosed $$ as text while a reply streams", () => {
    expect(blocks("$$\nE = mc")[0].kind).toBe("paragraph");
  });

  it("turns lines that are only images into image blocks", () => {
    expect(blocks("![shot](docs/shot.png)\n![b](b.png)")).toEqual([
      { kind: "image", alt: "shot", src: "docs/shot.png" },
      { kind: "image", alt: "b", src: "b.png" },
    ]);
  });

  it("keeps an image inside a sentence inline", () => {
    const [paragraph] = blocks("see ![shot](a.png) here");
    expect(paragraph.kind).toBe("paragraph");
    expect(paragraph.kind === "paragraph" && paragraph.inline.some((n) => n.type === "image")).toBe(true);
  });
});

describe("inline", () => {
  const nodes = (text: string, mentions = false) => parseInlineNodes(text, mentions);

  it("finds math, paths and the prose around them", () => {
    const parsed = nodes("Take $E=mc^2$ and open src/a.ts:12 please");
    expect(parsed.map((n) => n.type)).toEqual(["text", "math", "text", "path", "text"]);
    expect(parsed[3]).toMatchObject({ type: "path", ref: { path: "src/a.ts", line: 12 } });
  });

  it("makes a code span that is only a path into a chip, and other code stays code", () => {
    expect(nodes("`src/a.ts`")[0].type).toBe("path");
    expect(nodes("`npm test`")[0].type).toBe("code");
  });

  it("does not read underscores in a file name as italics", () => {
    const parsed = nodes("see src/my_file_name.ts now");
    expect(parsed.some((n) => n.type === "italic")).toBe(false);
    expect(parsed.some((n) => n.type === "path")).toBe(true);
  });

  it("still reads _italic_ and keeps prices out of math", () => {
    expect(nodes("an _emphasised_ word")[1].type).toBe("italic");
    expect(nodes("costs $5 and $10").every((n) => n.type === "text")).toBe(true);
  });

  it("finds @path mentions only for the user's own prompts", () => {
    expect(nodes("fix @src/a.ts", true).some((n) => n.type === "path")).toBe(true);
    expect(nodes("fix @src/a.ts", false).some((n) => n.type === "path")).toBe(false);
  });

  it("parses image and link syntax", () => {
    expect(nodes("![alt](x.png)")[0]).toEqual({ type: "image", alt: "alt", src: "x.png" });
    expect(nodes("[docs](https://example.com)")[0]).toEqual({ type: "link", text: "docs", href: "https://example.com" });
  });
});
