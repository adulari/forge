import { describe, expect, it, vi } from "vitest";

import { onScopeDenied, reportInputError, scopeDeniedMessage } from "./scopeDenied";

describe("scope denied notes", () => {
  it("notifies listeners only for scope_denied notes", () => {
    const seen = vi.fn();
    const off = onScopeDenied(seen);
    reportInputError("input ignored: bad json");
    reportInputError(undefined);
    reportInputError("scope_denied:approve");
    off();
    reportInputError("scope_denied:prompt");
    expect(seen).toHaveBeenCalledTimes(1);
    expect(seen).toHaveBeenCalledWith("approve");
  });

  it("phrases the refusal by scope", () => {
    expect(scopeDeniedMessage("approve")).toBe("This connection can view but not approve or deny requests.");
    expect(scopeDeniedMessage("whatever")).toBe("This connection can view but not do that.");
  });
});
