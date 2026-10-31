import { describe, expect, it, vi } from "vitest";

import { defaultPaneNav, secondaryPaneNav } from "./paneNav";

const routed = vi.hoisted(() => ({ openSession: vi.fn(), push: vi.fn() }));
vi.mock("expo-router", () => ({ router: { push: routed.push } }));
vi.mock("../../lib/nav", () => ({ openSession: routed.openSession }));

describe("pane navigation", () => {
  it("a secondary pane opens a session in itself and never routes pane 0", () => {
    const workbench = { openSession: vi.fn(), focusPane: vi.fn(), openSurface: vi.fn() };
    secondaryPaneNav(workbench, 2).openSession("child");
    expect(workbench.openSession).toHaveBeenCalledWith("child", { pane: 2, tap: true });
    expect(routed.openSession).not.toHaveBeenCalled();
    expect(routed.push).not.toHaveBeenCalled();
  });

  it("a secondary pane opens agents in the dock for that pane, not on a route", () => {
    const workbench = { openSession: vi.fn(), focusPane: vi.fn(), openSurface: vi.fn() };
    secondaryPaneNav(workbench, 1).openAgents("s1");
    expect(workbench.focusPane).toHaveBeenCalledWith(1);
    expect(workbench.openSurface).toHaveBeenCalledWith({ kind: "agents" });
    expect(routed.push).not.toHaveBeenCalled();
  });

  it("with no provider (pane 0, phones) the default is the routed navigation", () => {
    defaultPaneNav.openSession("x");
    expect(routed.openSession).toHaveBeenCalledWith("x");
    defaultPaneNav.openAgents("s1");
    expect(routed.push).toHaveBeenCalledWith("/session/s1/agents");
  });
});
