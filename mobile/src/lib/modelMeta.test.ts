import { describe, expect, it } from "vitest";

import { modelMetaLine } from "./modelMeta";

describe("modelMetaLine", () => {
  it("says how a model is paid for, its context and coding score", () => {
    expect(modelMetaLine({ subscription: true, free: false, context_window: 200_000, benchmark_coding: 62.14 })).toBe(
      "subscription · 200K · code 62.1",
    );
    expect(modelMetaLine({ subscription: false, free: true, context_window: null, benchmark_coding: null })).toBe("free");
    expect(modelMetaLine({ subscription: false, free: false, context_window: 1_000_000 })).toBe("api · 1M");
  });
});
