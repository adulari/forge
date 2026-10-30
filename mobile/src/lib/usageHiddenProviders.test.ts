import { describe, expect, it } from "vitest";

import { parseHiddenProviders } from "./usageHiddenProviders";

describe("parseHiddenProviders", () => {
  it("keeps only strings and survives junk", () => {
    expect(parseHiddenProviders('["a",3,"b"]')).toEqual(["a", "b"]);
    expect(parseHiddenProviders("{")).toEqual([]);
    expect(parseHiddenProviders(null)).toEqual([]);
  });
});
