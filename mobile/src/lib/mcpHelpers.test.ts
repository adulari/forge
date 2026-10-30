import { describe, expect, it } from "vitest";

import { joinArgs, mcpStatusMeta, mcpToolSummary, splitArgs, toggleToolDisabled } from "./mcpHelpers";

describe("mcp management helpers", () => {
  it("maps daemon statuses to a label and tone, defaulting to not checked", () => {
    expect(mcpStatusMeta("failed")).toEqual({ label: "Failed", tone: "danger" });
    expect(mcpStatusMeta("connected").tone).toBe("success");
    expect(mcpStatusMeta(undefined).label).toBe("Not checked");
    expect(mcpStatusMeta("from-the-future" as never).label).toBe("Not checked");
  });

  it("summarises tool counts with the number switched off", () => {
    expect(mcpToolSummary({ tool_count: null })).toBeNull();
    expect(mcpToolSummary({ tool_count: 1 })).toBe("1 tool");
    expect(mcpToolSummary({ tool_count: 12, disabled_tools: ["a", "b"] })).toBe("12 tools · 2 off");
  });

  it("toggles a tool in a sorted, duplicate-free disabled list", () => {
    expect(toggleToolDisabled(["b"], "a", false)).toEqual(["a", "b"]);
    expect(toggleToolDisabled(["a", "b"], "a", true)).toEqual(["b"]);
    expect(toggleToolDisabled(["a"], "a", false)).toEqual(["a"]);
  });

  it("round-trips arguments that contain spaces", () => {
    const args = ["--root", "/my dir", "--quiet"];
    expect(splitArgs(joinArgs(args))).toEqual(args);
    expect(splitArgs("  ")).toEqual([]);
  });
});
