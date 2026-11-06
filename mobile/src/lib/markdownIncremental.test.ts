import { describe, expect, it } from "vitest";

import { EMPTY_INCREMENTAL, type IncrementalParse, parseBlocksIncremental } from "./markdownIncremental";
import { parseBlocks } from "./markdownParse";

const SAMPLE = [
  "## Step 1: flaky auth test",
  "",
  "The daemon streams **full** snapshots, see `reconcile()` and [the docs](https://forge.adulari.dev/docs/1).",
  "A second line of the same paragraph.",
  "",
  "- first",
  "- second with `inline_code`",
  "",
  "1. One",
  "",
  "2. Two",
  "",
  "```rust",
  "fn a() {",
  "",
  "    let x = 1;",
  "",
  "}",
  "```",
  "",
  "> a quote",
  "",
  "| a | b |",
  "|---|---|",
  "| 1 | 2 |",
  "",
  "$$",
  "x^2",
  "",
  "+ y^2",
  "$$",
  "",
  "after the math",
  "   ",
  "tail with trailing space  ",
  "",
  "```",
  "unclosed fence",
  "",
  "still code",
].join("\n");

describe("parseBlocksIncremental", () => {
  it("equals a full parse for every prefix of a growing reply", () => {
    for (const mentions of [false, true]) {
      let state: IncrementalParse = EMPTY_INCREMENTAL;
      for (let n = 0; n <= SAMPLE.length; n++) {
        const content = SAMPLE.slice(0, n);
        const out = parseBlocksIncremental(content, mentions, state);
        state = out.state;
        expect(out.blocks).toEqual(parseBlocks(content, mentions));
      }
    }
  });

  it("starts over when the text is not an extension of what it settled", () => {
    const first = parseBlocksIncremental("one\n\ntwo\n\nthree", false, EMPTY_INCREMENTAL);
    expect(first.state.settled).not.toBe("");
    const other = parseBlocksIncremental("uno\n\ndos\n\ntres", false, first.state);
    expect(other.blocks).toEqual(parseBlocks("uno\n\ndos\n\ntres", false));
  });

  it("keeps finished blocks as the same objects between frames", () => {
    const a = parseBlocksIncremental("# head\n\npara one\n\npara tw", false, EMPTY_INCREMENTAL);
    const b = parseBlocksIncremental("# head\n\npara one\n\npara two", false, a.state);
    expect(b.blocks[0]).toBe(a.blocks[0]);
    expect(b.blocks[1]).toBe(a.blocks[1]);
    expect(b.blocks[2]).not.toBe(a.blocks[2]);
  });

  it("never settles inside a code fence", () => {
    const out = parseBlocksIncremental("text\n\n```ts\nlet a;\n\nlet b;", false, EMPTY_INCREMENTAL);
    expect(out.state.settled).toBe("text\n");
    expect(out.blocks).toEqual(parseBlocks("text\n\n```ts\nlet a;\n\nlet b;", false));
  });

  it("falls back to a plain parse for CRLF text", () => {
    const text = "a\r\n\r\nb";
    expect(parseBlocksIncremental(text, false, EMPTY_INCREMENTAL).blocks).toEqual(parseBlocks(text, false));
  });
});
