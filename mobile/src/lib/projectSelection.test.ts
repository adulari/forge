import { describe, expect, it } from "vitest";

import { isLoopbackServer, lastProjectStorageKey, projectChoices, projectName } from "./projectSelection";

describe("project selection", () => {
  it("uses one server-scoped remembered-project key", () => {
    expect(lastProjectStorageKey("srv_123")).toBe("forge.lastProject.srv_123");
  });

  it("recognizes local desktop daemons without treating remote hosts as local", () => {
    expect(isLoopbackServer("http://127.0.0.1:7420/token")).toBe(true);
    expect(isLoopbackServer("http://localhost:7420/token")).toBe(true);
    expect(isLoopbackServer("https://forge.example.com/token")).toBe(false);
  });

  it("deduplicates the default from recent projects and names cross-platform paths", () => {
    const choices = projectChoices("/work/forge", [
      { path: "/work/forge", name: "forge", is_git_repo: true, last_activity: 2 },
      { path: "/work/helm", name: "helm", is_git_repo: true, last_activity: 1 },
    ]);
    expect(choices.map((choice) => choice.path)).toEqual(["/work/forge", "/work/helm"]);
    expect(projectName("C:\\Users\\me\\forge\\")).toBe("forge");
  });
});
