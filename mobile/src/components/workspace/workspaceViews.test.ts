import { describe, expect, it } from "vitest";

import { gutterWidth, tokensByLine } from "./codeLines";
import { defaultViewMode, fileViewKind, highlightLanguage } from "./fileKind";
import { collapseWithin, creationDirectory, expandTo, loadExpanded, saveExpanded, toggleExpanded } from "./treeState";
import { isWorkspaceDirty, setWorkspaceDirty } from "./workspaceDirty";
import { ancestorDirectories } from "./workspaceReveal";

describe("fileViewKind", () => {
  it("routes svg, pdf and markdown to their viewers and everything else to text", () => {
    expect(fileViewKind("assets/logo.SVG")).toBe("svg");
    expect(fileViewKind("docs/a.pdf")).toBe("pdf");
    expect(fileViewKind("README.md")).toBe("markdown");
    expect(fileViewKind("notes.mdx")).toBe("markdown");
    expect(fileViewKind("src/main.rs")).toBe("text");
    expect(fileViewKind(".gitignore")).toBe("text");
    expect(highlightLanguage("src/main.rs")).toBe("rs");
  });

  it("opens rendered kinds rendered unless a line was asked for", () => {
    expect(defaultViewMode("markdown", null, true)).toBe("preview");
    expect(defaultViewMode("markdown", 12, true)).toBe("edit");
    expect(defaultViewMode("svg", null, true)).toBe("preview");
    expect(defaultViewMode("svg", null, false)).toBe("edit");
    expect(defaultViewMode("text", null, true)).toBe("edit");
  });
});

describe("tokensByLine", () => {
  it("cuts multi-line tokens so every line carries its own spans", () => {
    const lines = tokensByLine("let a = 1;\n/* x\ny */\nfn b() {}", "rs");
    expect(lines).toHaveLength(4);
    expect(lines[0]?.some((token) => token.kind === "keyword" && token.text === "let")).toBe(true);
    expect(lines[1]?.map((token) => token.kind)).toEqual(["comment"]);
    expect(lines[2]?.map((token) => token.kind)).toEqual(["comment"]);
    expect(lines.map((line) => line.map((token) => token.text).join("")).join("\n")).toBe(
      "let a = 1;\n/* x\ny */\nfn b() {}",
    );
  });

  it("keeps blank lines and sizes the gutter", () => {
    expect(tokensByLine("a\n\nb", "txt")).toEqual([
      [{ kind: "plain", text: "a" }],
      [],
      [{ kind: "plain", text: "b" }],
    ]);
    expect(gutterWidth(9)).toBe(2);
    expect(gutterWidth(12345)).toBe(5);
  });
});

describe("tree state", () => {
  it("expands every ancestor of a revealed file", () => {
    expect(ancestorDirectories("a/b/c.ts")).toEqual(["a", "a/b"]);
    expect([...expandTo(new Set(), "a/b/c.ts")].sort()).toEqual(["a", "a/b"]);
    expect([...expandTo(new Set(["z"]), "top.ts")]).toEqual(["z"]);
  });

  it("toggles and folds nested folders away", () => {
    const open = new Set(["a", "a/b", "ab", "c"]);
    expect([...toggleExpanded(open, "c")].sort()).toEqual(["a", "a/b", "ab"]);
    expect([...collapseWithin(open, "a")].sort()).toEqual(["ab", "c"]);
  });

  it("remembers open folders per session while the browser tab is away", () => {
    saveExpanded("s1", new Set(["a", "a/b"]));
    expect([...loadExpanded("s1")]).toEqual(["a", "a/b"]);
    expect(loadExpanded("s2").size).toBe(0);
    saveExpanded("s1", new Set());
    expect(loadExpanded("s1").size).toBe(0);
  });

  it("creates in the selected folder or beside the selected file", () => {
    expect(creationDirectory(null)).toBe("");
    expect(creationDirectory({ path: "src/ui", isDirectory: true })).toBe("src/ui");
    expect(creationDirectory({ path: "src/ui/a.ts", isDirectory: false })).toBe("src/ui");
    expect(creationDirectory({ path: "a.ts", isDirectory: false })).toBe("");
  });
});

describe("workspace dirty store", () => {
  it("tracks unsaved files per session", () => {
    setWorkspaceDirty("s1", "a.ts", true);
    expect(isWorkspaceDirty("s1", "a.ts")).toBe(true);
    expect(isWorkspaceDirty("s2", "a.ts")).toBe(false);
    setWorkspaceDirty("s1", "a.ts", false);
    expect(isWorkspaceDirty("s1", "a.ts")).toBe(false);
  });
});

describe("reveal requests", () => {
  it("applies a request once and not again on remount", async () => {
    const { markRevealHandled, pendingReveal } = await import("./workspaceReveal");
    const request = { sessionId: "s9", path: "a/b.ts", nonce: 7 };
    expect(pendingReveal(request, "s9")).toBe(request);
    expect(pendingReveal(request, "other")).toBeNull();
    markRevealHandled(request);
    expect(pendingReveal(request, "s9")).toBeNull();
    expect(pendingReveal({ ...request, nonce: 8 }, "s9")).not.toBeNull();
  });
});
