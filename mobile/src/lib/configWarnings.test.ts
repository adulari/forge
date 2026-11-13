import { describe, expect, it } from "vitest";

import type { ConfigResponse } from "./api";
import { configWarning } from "./configWarnings";

const response: ConfigResponse = {
  fields: [],
  warnings: [
    { key: "harness.expert.model", text: "codex-oauth::gpt-5.6-sol needs a budget: set harness.expert.budget" },
    { key: "harness.expert.budget", text: "codex-oauth::gpt-5.6-sol needs a budget: set harness.expert.budget" },
  ],
};

describe("configWarning", () => {
  it("finds the warning for the expert model and budget rows", () => {
    expect(configWarning(response, "harness.expert.budget")).toContain("set harness.expert.budget");
    expect(configWarning(response, "harness.expert.model")).not.toBeNull();
  });

  it("is null for other fields, older daemons and an unloaded page", () => {
    expect(configWarning(response, "tui.theme")).toBeNull();
    expect(configWarning({ fields: [] }, "harness.expert.model")).toBeNull();
    expect(configWarning(undefined, "harness.expert.model")).toBeNull();
  });
});
