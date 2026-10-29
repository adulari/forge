import { describe, expect, it } from "vitest";

import { tileLayout } from "./tileLayoutModel";

describe("tileLayout", () => {
  it("fits as many columns of at least 150 as the width allows", () => {
    expect(tileLayout(336)).toEqual({ columns: 2, tile: 164 });
    expect(tileLayout(150)).toEqual({ columns: 1, tile: 150 });
    expect(tileLayout(500).columns).toBe(3);
    expect(tileLayout(100).columns).toBe(1);
  });
});
