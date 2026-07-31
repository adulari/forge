import { describe, expect, it } from "vitest";

import {
  parentWorkspacePath,
  replaceWorkspaceMention,
  workspaceBasename,
  workspaceMentionAtEnd,
  workspaceMentionToken,
} from "./workspaceModel";

describe("workspace UI model", () => {
  it("navigates parent paths without escaping the root", () => {
    expect(parentWorkspacePath("src/components/chat")).toBe("src/components");
    expect(parentWorkspacePath("src")).toBe("");
    expect(parentWorkspacePath("")).toBe("");
  });

  it("extracts a trailing composer file mention", () => {
    expect(workspaceMentionAtEnd("review @src/com")).toEqual({
      start: 7,
      query: "src/com",
    });
    expect(workspaceMentionAtEnd("@")).toEqual({ start: 0, query: "" });
    expect(workspaceMentionAtEnd("email me@example.com")).toBeNull();
    expect(workspaceMentionAtEnd("review @src/a then")).toBeNull();
  });

  it("replaces only the active mention and braces paths containing spaces", () => {
    const mention = workspaceMentionAtEnd("review @src")!;
    expect(replaceWorkspaceMention("review @src", mention, "src/main.rs")).toBe(
      "review @src/main.rs ",
    );
    expect(workspaceMentionToken("docs/My Plan.md")).toBe("@{docs/My Plan.md}");
    expect(workspaceBasename("docs/My Plan.md")).toBe("My Plan.md");
  });
});
