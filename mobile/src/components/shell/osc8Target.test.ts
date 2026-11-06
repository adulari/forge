import { describe, expect, it } from "vitest";

import { osc8Target } from "./terminalLinks";

describe("osc8Target", () => {
  it("opens web and file links", () => {
    expect(osc8Target("https://example.com")).toEqual({ kind: "url", url: "https://example.com" });
    expect(osc8Target("file:///repo/src/a%20b.rs")).toEqual({
      kind: "path",
      ref: { path: "/repo/src/a b.rs", line: null, column: null },
    });
  });

  it("ignores a malformed escape from program output instead of throwing", () => {
    expect(osc8Target("file:///%zz")).toBeNull();
  });
});
