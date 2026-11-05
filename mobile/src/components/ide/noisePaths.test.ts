import { describe, expect, it } from "vitest";

import { isNoisePath } from "./noisePaths";

describe("isNoisePath", () => {
  it("flags build and cache residue", () => {
    for (const path of [
      "a/__pycache__/x.txt",
      "m.pyc",
      "a/b.pyo",
      ".DS_Store",
      "d/.DS_Store",
      ".pytest_cache/v/cache",
      ".mypy_cache/x",
      ".ruff_cache/x",
      "web/node_modules/p/index.js",
      "f.swp",
    ]) {
      expect(isNoisePath(path), path).toBe(true);
    }
  });

  it("keeps real files", () => {
    for (const path of ["src/main.rs", "pycache.txt", "node_modules", "a/b.pyc.md", "notes.swpx"]) {
      expect(isNoisePath(path), path).toBe(false);
    }
  });
});
