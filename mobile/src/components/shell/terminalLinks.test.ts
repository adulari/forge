import { describe, expect, it } from "vitest";

import { detectTerminalLinks } from "./terminalLinks";

const links = (text: string) =>
  detectTerminalLinks(text).flatMap((part) => (part.type === "link" ? [part.target] : []));

describe("terminal link detection", () => {
  it("links http(s) URLs and leaves sentence punctuation outside", () => {
    expect(links("Local: http://localhost:5173/, ready")).toEqual([
      { kind: "url", url: "http://localhost:5173/" },
    ]);
    expect(links("see (https://example.com/a_(b)).")).toEqual([
      { kind: "url", url: "https://example.com/a_(b)" },
    ]);
    expect(links("bare https:// is nothing")).toEqual([]);
  });

  it("links file:line paths with their line and column", () => {
    expect(links("error at src/a.ts:5:12 in build")).toEqual([
      { kind: "path", ref: { path: "src/a.ts", line: 5, column: 12 } },
    ]);
    expect(links("crates/forge-cli/src/serve.rs:310")).toEqual([
      { kind: "path", ref: { path: "crates/forge-cli/src/serve.rs", line: 310, column: null } },
    ]);
  });

  it("leaves a bare listing name alone but links a path inside a folder", () => {
    expect(links("Cargo.toml  README.md")).toEqual([]);
    expect(links("modified: src/lib/a.ts")).toEqual([
      { kind: "path", ref: { path: "src/lib/a.ts", line: null, column: null } },
    ]);
  });

  it("does not read a path out of a URL and keeps the surrounding text in order", () => {
    const parts = detectTerminalLinks("open http://x.dev/src/a.ts:5 then src/b.ts:7!");
    expect(parts.map((part) => part.text).join("")).toBe("open http://x.dev/src/a.ts:5 then src/b.ts:7!");
    expect(parts.filter((part) => part.type === "link").map((part) => part.text)).toEqual([
      "http://x.dev/src/a.ts:5",
      "src/b.ts:7",
    ]);
  });

  it("returns plain text untouched", () => {
    expect(detectTerminalLinks("hello world")).toEqual([{ type: "text", text: "hello world" }]);
    expect(detectTerminalLinks("")).toEqual([{ type: "text", text: "" }]);
  });
});
