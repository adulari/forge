import { describe, expect, it } from "vitest";

import { appendOutput, checkActions, checkState, failingChangedFiles, fixPrompt, OUTPUT_LIMIT, parseFileRefs, resolveRefPath, splitLineRefs, stripAnsi, tailLines } from "./checksModel";

describe("checkState", () => {
  it("maps run state and exit code", () => {
    expect(checkState({ running: true, exit_code: null, terminal_id: "t" })).toBe("running");
    expect(checkState({ running: false, exit_code: 0, terminal_id: "t" })).toBe("pass");
    expect(checkState({ running: false, exit_code: 101, terminal_id: "t" })).toBe("fail");
    expect(checkState({ running: false, exit_code: null, terminal_id: null })).toBe("idle");
  });
});

describe("parseFileRefs", () => {
  it("finds rustc, pytest, tsc and plain refs and strips colour", () => {
    const text = "\u001b[31merror\u001b[0m: boom\n  --> src/lib.rs:12:5\nFAILED tests/test_a.py:44: assert\nsrc/ui/App.tsx(7,3): error TS2304\n./pkg/util.go:9:2: undefined";
    expect(parseFileRefs(text)).toEqual([
      { path: "src/lib.rs", line: 12, column: 5 },
      { path: "tests/test_a.py", line: 44, column: null },
      { path: "src/ui/App.tsx", line: 7, column: 3 },
      { path: "pkg/util.go", line: 9, column: 2 },
    ]);
  });
  it("ignores versions and hosts, and dedupes", () => {
    expect(parseFileRefs("node v20.1:3 localhost:8080\na.rs:1\na.rs:1")).toEqual([{ path: "a.rs", line: 1, column: null }]);
  });
});

describe("failing files", () => {
  const refs = parseFileRefs("--> crates/x/src/lib.rs:3:1\nother.rs:9");
  it("matches a changed file by path suffix", () => {
    expect(failingChangedFiles(["crates/x/src/lib.rs", "README.md"], refs)).toEqual(["crates/x/src/lib.rs"]);
    expect(failingChangedFiles(["src/lib.rs"], refs)).toEqual(["src/lib.rs"]);
  });
  it("resolves a ref to the changed path", () => {
    expect(resolveRefPath({ path: "src/lib.rs", line: 1, column: null }, ["crates/x/src/lib.rs"])).toBe("crates/x/src/lib.rs");
    expect(resolveRefPath({ path: "z.rs", line: 1, column: null }, [])).toBe("z.rs");
  });
});

describe("fixPrompt", () => {
  it("names the check, the exit code and the tail", () => {
    const out = Array.from({ length: 100 }, (_, i) => `l${i}`).join("\n");
    const prompt = fixPrompt("test", "cargo test", 101, out);
    expect(prompt).toContain('"test" check failed (exit 101)');
    expect(prompt).toContain("l99");
    expect(prompt).not.toContain("l10\n");
    expect(tailLines(out, 2)).toBe("l98\nl99");
  });
  it("strips escapes", () => {
    expect(stripAnsi("\u001b[1mbold\u001b[0m")).toBe("bold");
  });
});


describe("splitLineRefs", () => {
  it("makes the first ref a link and keeps the rest of the line", () => {
    const segments = splitLineRefs("  --> src/lib.rs:12:5 (note)");
    expect(segments.map((s) => s.text)).toEqual(["  --> ", "src/lib.rs:12:5", " (note)"]);
    expect(segments[1].ref).toEqual({ path: "src/lib.rs", line: 12, column: 5 });
    expect(splitLineRefs("plain line")).toEqual([{ text: "plain line", ref: null }]);
  });
  it("handles the tsc form", () => {
    const segments = splitLineRefs("src/a.ts(7,3): error");
    expect(segments[0].text).toBe("src/a.ts(7,3)");
    expect(segments[0].ref?.line).toBe(7);
  });
});

describe("checkActions", () => {
  const a = (id: string, command = "x") => ({ id, name: id, command });
  it("keeps test, lint and build actions", () => {
    expect(checkActions([a("dev"), a("test"), a("lint"), a("start")]).map((x) => x.id)).toEqual(["test", "lint"]);
  });
  it("falls back to every action when none look like a check", () => {
    expect(checkActions([a("dev"), a("start")]).map((x) => x.id)).toEqual(["dev", "start"]);
  });
});

describe("appendOutput", () => {
  it("keeps the newest bytes", () => {
    const big = appendOutput("a".repeat(OUTPUT_LIMIT), "tail");
    expect(big.length).toBe(OUTPUT_LIMIT);
    expect(big.endsWith("tail")).toBe(true);
  });
});
