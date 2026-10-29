// Every workbench command in one place: panes, docks and layout presets. Each is claimed twice,
// as a persisted keyboard shortcut and as a menu id (`useDesktopMenuAction`), so the macOS menu
// bar, the title-bar Layout menu, the command palette and the keyboard all run the same code.
// Inert while `enabled` is false (compact windows, where none of this chrome renders).
import { router } from "expo-router";

import { useEffect } from "react";

import { useAppShortcut, type AppShortcutAction } from "../../lib/shortcuts";
import { useKeybindingsSync } from "../../lib/shortcuts/useKeybindingsSync";
import { publishFocusTag } from "../../lib/shortcuts/focusContext";
import { useDesktopMenuAction, type DesktopMenuId } from "../../lib/desktopMenu";
import type { PresetName } from "./layoutModel";
import type { WorkbenchSurfaceKind } from "./model";
import { useWorkbench } from "./WorkbenchProvider";

export interface WorkbenchCommandOptions {
  enabled: boolean;
  /** Sidebar state lives in the root layout (MasterDetail sizes its rail from it). */
  setSidebarCollapsed: (collapsed: boolean) => void;
}

function useCommand(action: AppShortcutAction, menu: DesktopMenuId, run: () => void): void {
  useAppShortcut(action, run);
  useDesktopMenuAction(menu, run);
}

export function useWorkbenchCommands({ enabled, setSidebarCollapsed }: WorkbenchCommandOptions): void {
  const workbench = useWorkbench();
  const guard = (fn: () => void) => () => {
    if (enabled) fn();
  };
  const toggle = (kind: WorkbenchSurfaceKind) => guard(() => workbench.toggleSurface({ kind }));
  const preset = (name: PresetName, sidebarCollapsed: boolean | null) =>
    guard(() => {
      workbench.applyPreset(name);
      if (sidebarCollapsed != null) setSidebarCollapsed(sidebarCollapsed);
    });
  const { layout, focusedSessionId } = workbench;
  const total = 1 + layout.secondary.length;
  useKeybindingsSync();
  const hasSession = focusedSessionId != null;
  useEffect(() => {
    publishFocusTag("hasSession", hasSession);
    return () => publishFocusTag("hasSession", false);
  }, [hasSession]);

  useCommand("workbench.split", "view:split-pane", guard(() => workbench.addPane()));
  useCommand("workbench.closePane", "view:pane-close", guard(() => {
    if (total > 1) workbench.closePane(layout.focus);
  }));
  useCommand("workbench.maximize", "view:pane-maximize", guard(() => workbench.toggleMaximize()));
  useCommand("workbench.paneNext", "view:pane-next", guard(() => workbench.cycleFocus(1)));
  useCommand("workbench.panePrev", "view:pane-prev", guard(() => workbench.cycleFocus(-1)));

  useCommand("workbench.terminal", "view:terminal", toggle("terminal"));
  useCommand("workbench.gitReview", "view:git-review", toggle("git"));
  useCommand("workbench.tasks", "view:tasks", toggle("tasks"));
  useCommand("workbench.agents", "view:agents", toggle("agents"));
  useCommand("workbench.plan", "view:plan", toggle("plan"));
  useCommand("workbench.files", "view:files", toggle("files"));
  useCommand("workbench.preview", "view:browser-preview", guard(() => {
    if (focusedSessionId) workbench.toggleSurface({ kind: "preview", sessionId: focusedSessionId });
  }));
  useAppShortcut("session.next", guard(() => workbench.stepSession(1)));
  useAppShortcut("session.prev", guard(() => workbench.stepSession(-1)));
  useAppShortcut("session.recent", guard(() => workbench.cycleRecent()));
  useAppShortcut("settings", () => router.push("/settings"));
  useAppShortcut("session.close", guard(() => {
    if (total > 1) workbench.closePane(layout.focus);
    else if (focusedSessionId) router.replace("/");
  }));
  useDesktopMenuAction("view:gallery", toggle("gallery"));

  useDesktopMenuAction("view:layout-focus", preset("focus", true));
  useDesktopMenuAction("view:layout-side-by-side", preset("side-by-side", false));
  useDesktopMenuAction("view:layout-grid", preset("grid", true));
  useDesktopMenuAction("view:layout-review", preset("review", false));
  useDesktopMenuAction(
    "view:layout-reset",
    guard(() => {
      workbench.resetLayout();
      setSidebarCollapsed(false);
    }),
  );
}
