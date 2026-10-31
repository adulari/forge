import { describe, expect, it } from "vitest";

import { withoutUnsupportedFields } from "./createSessionCompat";

class ApiError extends Error {
  constructor(public status: number, message: string) {
    super(message);
  }
}

describe("withoutUnsupportedFields", () => {
  const body = { cwd: "/r", worktree: true, branch: "x", effort: "high", title: "t" };
  it("strips branch and effort for an older daemon", () => {
    const error = new ApiError(400, "unknown field `branch`, expected one of `cwd`");
    expect(withoutUnsupportedFields(body, error)).toEqual({ cwd: "/r", worktree: true, title: "t" });
  });
  it("leaves other failures alone", () => {
    expect(withoutUnsupportedFields(body, new ApiError(400, "not a git repository"))).toBeNull();
    expect(withoutUnsupportedFields(body, new ApiError(400, "unknown field `prompt`"))).toBeNull();
    expect(withoutUnsupportedFields(body, new Error("x"))).toBeNull();
  });
});
