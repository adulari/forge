import { describe, expect, it } from "vitest";

import { MAX_RECENTS, pushRecent, recencyOf } from "./recents";

describe("recents", () => {
  it("moves a rerun entry to the front without duplicating it", () => {
    expect(pushRecent(["a", "b", "c"], "b")).toEqual(["b", "a", "c"]);
  });

  it("keeps only the newest entries", () => {
    const full = Array.from({ length: MAX_RECENTS }, (_, i) => `id${i}`);
    const next = pushRecent(full, "fresh");
    expect(next).toHaveLength(MAX_RECENTS);
    expect(next[0]).toBe("fresh");
    expect(next).not.toContain(`id${MAX_RECENTS - 1}`);
  });

  it("ranks newer entries higher and unknown entries at zero", () => {
    const list = ["new", "old"];
    expect(recencyOf(list, "new")).toBeGreaterThan(recencyOf(list, "old"));
    expect(recencyOf(list, "never")).toBe(0);
  });
});
