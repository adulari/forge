import { describe, expect, it } from "vitest";

import { displayPermissionPrompt } from "./permissionPrompt";

describe("displayPermissionPrompt", () => {
  it("drops the terminal's key hint and asks the question", () => {
    expect(displayPermissionPrompt("allow write_file (Write) [y/n]")).toBe("Allow write_file (Write)?");
    expect(displayPermissionPrompt("allow shell (Shell) [n]")).toBe("Allow shell (Shell)?");
  });

  it("leaves a prompt without a hint readable", () => {
    expect(displayPermissionPrompt("Run the migration?")).toBe("Run the migration?");
    expect(displayPermissionPrompt("[y/n]")).toBe("[y/n]");
  });
});
