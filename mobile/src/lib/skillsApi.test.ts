import { describe, expect, it, vi } from "vitest";

import { filterAvailable, getSkillsState, installSkill, invokeSkill, syncSkills } from "./skillsApi";

const { request } = vi.hoisted(() => ({ request: vi.fn() }));
vi.mock("./api", () => ({ request }));

describe("skills api", () => {
  it("filters marketplace packages on name and description", () => {
    const rows = [
      { name: "rust-helper", description: "Borrow checker coaching" },
      { name: "deploy", description: "Ship to prod" },
    ];
    expect(filterAvailable(rows, "")).toHaveLength(2);
    expect(filterAvailable(rows, "BORROW").map((r) => r.name)).toEqual(["rust-helper"]);
    expect(filterAvailable(rows, "nothing")).toEqual([]);
  });

  it("posts installs with scope and marketplace", async () => {
    request.mockResolvedValue({});
    await installSkill("http://h", { source: "pack", scope: "project", marketplace: "official", session: "s1" });
    const [, path, init] = request.mock.calls.at(-1) as [string, string, RequestInit];
    expect(path).toBe("/api/skills/install");
    expect(init.method).toBe("POST");
    expect(JSON.parse(init.body as string)).toEqual({ source: "pack", scope: "project", marketplace: "official", session: "s1" });
  });

  it("addresses project state, sync and invoke by session", async () => {
    request.mockResolvedValue({});
    await getSkillsState("http://h", "a b");
    expect(request.mock.calls.at(-1)?.[1]).toBe("/api/skills/state?session=a%20b");
    await syncSkills("http://h", { scope: "user" });
    expect(request.mock.calls.at(-1)?.[1]).toBe("/api/skills/sync");
    await invokeSkill("http://h", { name: "rust", task: "fix" });
    expect(request.mock.calls.at(-1)?.[1]).toBe("/api/skills/invoke");
  });
});
