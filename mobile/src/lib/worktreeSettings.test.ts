import { describe, expect, it } from "vitest";

import {
  commandToSave,
  globProblem,
  normalizeRoot,
  parseGlobList,
  serializeGlobList,
  withGlob,
  withoutGlob,
} from "./worktreeSettings";

describe("glob list", () => {
  it("round-trips through JSON text and survives junk", () => {
    expect(parseGlobList(serializeGlobList([".env*", ".vscode"]))).toEqual([".env*", ".vscode"]);
    expect(parseGlobList("not json")).toEqual([]);
    expect(parseGlobList('{"a":1}')).toEqual([]);
    expect(parseGlobList('[".env", 3]')).toEqual([".env"]);
  });
  it("adds trimmed, unique patterns and removes by index", () => {
    expect(withGlob([".env*"], "  .vscode ")).toEqual([".env*", ".vscode"]);
    expect(withGlob([".env*"], ".env*")).toEqual([".env*"]);
    expect(withGlob([".env*"], "  ")).toEqual([".env*"]);
    expect(withoutGlob(["a", "b", "c"], 1)).toEqual(["a", "c"]);
  });
});

describe("globProblem", () => {
  it("flags patterns that can never match inside the repo", () => {
    expect(globProblem(".env*")).toBeNull();
    expect(globProblem("config/*.local")).toBeNull();
    expect(globProblem("")).toBeNull();
    expect(globProblem("/etc/passwd")).toMatch(/relative/);
    expect(globProblem("C:\\x")).toMatch(/relative/);
    expect(globProblem("../secrets")).toMatch(/leave/);
  });
});

describe("root and commands", () => {
  it("normalizes the root value", () => {
    expect(normalizeRoot("global")).toBe("global");
    expect(normalizeRoot(" Global ")).toBe("global");
    expect(normalizeRoot("")).toBe("project");
    expect(normalizeRoot("anything")).toBe("project");
  });
  it("clears blank commands", () => {
    expect(commandToSave("  npm ci  ")).toBe("npm ci");
    expect(commandToSave("   ")).toBeUndefined();
  });
});
