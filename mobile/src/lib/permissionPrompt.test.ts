import { describe, expect, it } from "vitest";

import { displayPermissionPrompt, permissionPromptParts } from "./permissionPrompt";

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

describe("permissionPromptParts", () => {
  it("splits the call's subject off the question", () => {
    expect(permissionPromptParts("allow shell (Shell): python3 -m pytest -q [y/n]")).toEqual({
      question: "Allow shell (Shell)?",
      subject: "python3 -m pytest -q",
    });
    expect(displayPermissionPrompt("allow shell (Shell): ls [y/n]")).toBe("Allow shell (Shell)? ls");
  });

  it("has no subject for the bare line", () => {
    expect(permissionPromptParts("allow write_file (Write) [y/n]")).toEqual({ question: "Allow write_file (Write)?", subject: null });
  });
});
