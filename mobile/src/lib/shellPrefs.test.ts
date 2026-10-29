import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  activeWorkspaceProjects,
  clampSidebarWidth,
  createWorkspace,
  deleteWorkspace,
  parseShellPrefs,
  readShellPrefs,
  resetShellPrefsForTest,
  setActiveWorkspace,
  setSidebarWidth,
  toggleGroupCollapsed,
  toggleProjectInWorkspace,
} from "./shellPrefs";

const store = new Map<string, string>();
vi.mock("@react-native-async-storage/async-storage", () => ({
  default: {
    getItem: vi.fn(async (key: string) => store.get(key) ?? null),
    setItem: vi.fn(async (key: string, value: string) => void store.set(key, value)),
  },
}));

beforeEach(() => {
  store.clear();
  resetShellPrefsForTest();
});

describe("parseShellPrefs", () => {
  it("falls back to defaults for nothing or garbage", () => {
    expect(parseShellPrefs(null).sort).toBe("updated");
    expect(parseShellPrefs("{not json").sidebarWidth).toBe(232);
  });

  it("clamps the width and drops an active workspace that no longer exists", () => {
    const parsed = parseShellPrefs(
      JSON.stringify({
        sidebarWidth: 900,
        activeWorkspaceId: "gone",
        workspaces: [{ id: "a", name: "Work", projects: ["forge", 3] }, { id: 4 }],
        collapsedGroups: ["x", 1],
        sort: "created",
      }),
    );
    expect(parsed.sidebarWidth).toBe(360);
    expect(parsed.activeWorkspaceId).toBeNull();
    expect(parsed.workspaces).toEqual([{ id: "a", name: "Work", projects: ["forge"] }]);
    expect(parsed.collapsedGroups).toEqual(["x"]);
    expect(parsed.sort).toBe("created");
  });
});

describe("shell prefs writes", () => {
  it("clamps widths into the 200 to 360 range", () => {
    expect(clampSidebarWidth(120)).toBe(200);
    expect(clampSidebarWidth(250.4)).toBe(250);
    setSidebarWidth(1000);
    expect(readShellPrefs().sidebarWidth).toBe(360);
  });

  it("persists a toggled group and toggles it back", () => {
    toggleGroupCollapsed("k");
    expect(JSON.parse(store.get("forge.shell.v1") as string).collapsedGroups).toEqual(["k"]);
    toggleGroupCollapsed("k");
    expect(readShellPrefs().collapsedGroups).toEqual([]);
  });

  it("scopes a workspace to its projects", () => {
    expect(createWorkspace("  ")).toBeNull();
    const id = createWorkspace("Work") as string;
    expect(readShellPrefs().activeWorkspaceId).toBe(id);
    toggleProjectInWorkspace(id, "forge");
    expect([...(activeWorkspaceProjects(readShellPrefs()) ?? [])]).toEqual(["forge"]);
    setActiveWorkspace(null);
    expect(activeWorkspaceProjects(readShellPrefs())).toBeNull();
    setActiveWorkspace(id);
    deleteWorkspace(id);
    expect(readShellPrefs().activeWorkspaceId).toBeNull();
    expect(readShellPrefs().workspaces).toEqual([]);
  });
});
