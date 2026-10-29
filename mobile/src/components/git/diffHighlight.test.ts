import { describe, expect, it } from "vitest";

import { languageForPath, styledSpans } from "./diffHighlight";

describe("languageForPath", () => {
  it("maps the extensions the tokenizer knows", () => {
    expect(languageForPath("crates/a/src/lib.rs")).toBe("rust");
    expect(languageForPath("app/Foo.TSX")).toBe("js");
    expect(languageForPath("scripts/run.sh")).toBe("bash");
    expect(languageForPath("pkg/main.go")).toBe("go");
    expect(languageForPath("data.json")).toBe("json");
  });

  it("returns null for unknown extensions, dotfiles and extensionless names", () => {
    expect(languageForPath("README.md")).toBeNull();
    expect(languageForPath(".gitignore")).toBeNull();
    expect(languageForPath("Makefile")).toBeNull();
    expect(languageForPath("dir.d/file")).toBeNull();
  });
});

describe("styledSpans", () => {
  it("colours keywords, strings and numbers", () => {
    const spans = styledSpans('let x = "hi"; // note', "rust");
    expect(spans.map((span) => span.kind)).toContain("keyword");
    expect(spans.find((span) => span.kind === "string")?.text).toBe('"hi"');
    expect(spans.find((span) => span.kind === "comment")?.text).toBe("// note");
    expect(spans.map((span) => span.text).join("")).toBe('let x = "hi"; // note');
  });

  it("returns one plain span without a language", () => {
    expect(styledSpans("let x = 1", null)).toEqual([{ text: "let x = 1", kind: "plain", changed: false }]);
  });

  it("splits a token where an intraline change starts inside it", () => {
    const spans = styledSpans("return value", "js", [
      { text: "return va", changed: false },
      { text: "lue", changed: true },
    ]);
    expect(spans.map((span) => span.text).join("")).toBe("return value");
    expect(spans.find((span) => span.changed)?.text).toBe("lue");
    expect(spans[0]).toMatchObject({ text: "return", kind: "keyword", changed: false });
  });

  it("keeps every character exactly once however the boundaries fall", () => {
    const text = 'const n = 42; // x "y"';
    const segments = [
      { text: "const ", changed: false },
      { text: "n = 4", changed: true },
      { text: "2; // x ", changed: false },
      { text: '"y"', changed: true },
    ];
    expect(styledSpans(text, "js", segments).map((span) => span.text).join("")).toBe(text);
  });

  it("skips highlighting for very long lines", () => {
    const line = `let ${"a".repeat(700)} = 1`;
    const spans = styledSpans(line, "rust");
    expect(spans).toHaveLength(1);
    expect(spans[0].kind).toBe("plain");
  });

  it("handles the empty line", () => {
    expect(styledSpans("", "rust")).toEqual([{ text: "", kind: "plain", changed: false }]);
  });
});
