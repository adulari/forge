import { describe, expect, it } from "vitest";

import { type GitFileRow, type GitStatusResponse } from "../../lib/api";
import { diffTextSegments, toSplitRows, toUnifiedRows } from "./diffModel";
import { buildDiffEntries, CARD_RENDER_LIMIT, defaultExpandedKeys, EXPAND_ALL_LIMIT } from "./gitDiffEntries";

const row = (path: string, adds: number, dels = 0): GitFileRow => ({ path, status: "M", orig_path: null, adds, dels, binary: false });
const status = (unstaged: GitFileRow[]): GitStatusResponse => ({
  root: "/r",
  branch: "main",
  base_branch: null,
  staged: [],
  unstaged,
  untracked: [],
  truncated: 0,
});

describe("a large diff in the git review dock", () => {
  it("opens one card for a thousand-file change and leaves the rest closed", () => {
    const entries = buildDiffEntries(status(Array.from({ length: 1000 }, (_, i) => row(`src/f${i}.rs`, 1))));
    expect(entries).toHaveLength(1000);
    expect(defaultExpandedKeys(entries).size).toBeLessThanOrEqual(6);
    expect(CARD_RENDER_LIMIT).toBeLessThan(entries.length);
    expect(EXPAND_ALL_LIMIT).toBeLessThan(CARD_RENDER_LIMIT);
  });

  it("still opens the first card when it alone is enormous", () => {
    const entries = buildDiffEntries(status([row("generated.rs", 50_000, 50_000), row("small.rs", 1)]));
    expect([...defaultExpandedKeys(entries)]).toEqual(["unstaged:generated.rs"]);
  });

  it("lays out a 20,000-line hunk in both views without dropping a line", () => {
    const lines = Array.from({ length: 20_000 }, (_, i) => (i % 3 === 0 ? `-old ${i}` : i % 3 === 1 ? `+new ${i}` : ` same ${i}`));
    const hunks = [{ header: "@@ -1,20000 +1,20000 @@", lines }];
    const start = performance.now();
    const unified = toUnifiedRows(hunks);
    const split = toSplitRows(hunks);
    const elapsed = performance.now() - start;
    expect(unified.filter((r) => r.kind === "line")).toHaveLength(20_000);
    expect(split.length).toBeGreaterThan(0);
    expect(elapsed).toBeLessThan(2_000);
  });

  it("skips the word-level diff for a minified line instead of freezing on it", () => {
    const longOld = Array.from({ length: 5_000 }, (_, i) => `tok${i}`).join(" ");
    const longNew = `${longOld} extra`;
    const start = performance.now();
    const result = diffTextSegments(longOld, longNew);
    expect(performance.now() - start).toBeLessThan(500);
    expect(result.oldSegments.map((s) => s.text).join("")).toBe(longOld);
    expect(result.newSegments.map((s) => s.text).join("")).toBe(longNew);
  });
});
