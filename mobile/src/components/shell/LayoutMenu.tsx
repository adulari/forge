// The visible home of the workbench: a "Layout" button in the window title bar whose menu lists
// every pane, panel and preset with its keyboard shortcut. Everything here also exists as a
// shortcut, a macOS menu item and a command-palette entry — this is the one that needs no
// memorising. Every row dispatches through `runDesktopMenuAction`, so it runs exactly the code
// the shortcut runs.
import { ChevronDown, LayoutPanelLeft } from "lucide-react-native";
import React, { useMemo, useRef, useState } from "react";
import { Pressable, StyleSheet, Text, View, useWindowDimensions } from "react-native";

import { runDesktopMenuAction, type DesktopMenuId } from "../../lib/desktopMenu";
import { useShortcutLabel } from "../../lib/shortcuts";
import { useTokens } from "../../theme/ThemeProvider";
import { radii } from "../../theme/tokens";
import { monoFamily } from "../../theme/typography";
import { MAX_PANES } from "../workbench/layoutModel";
import { activeWorkbenchSurface, type WorkbenchSurfaceKind } from "../workbench/model";
import { useWorkbench } from "../workbench/WorkbenchProvider";
import { PopoverMenu, type MenuAnchor, type MenuEntry } from "./PopoverMenu";

export function useLayoutMenuEntries(): MenuEntry[] {
  const workbench = useWorkbench();
  const hints = {
    split: useShortcutLabel("workbench.split"),
    close: useShortcutLabel("workbench.closePane"),
    maximize: useShortcutLabel("workbench.maximize"),
    next: useShortcutLabel("workbench.paneNext"),
    prev: useShortcutLabel("workbench.panePrev"),
    sidebar: useShortcutLabel("shell.sidebar"),
    usage: useShortcutLabel("shell.usage"),
    tasks: useShortcutLabel("workbench.tasks"),
    agents: useShortcutLabel("workbench.agents"),
    plan: useShortcutLabel("workbench.plan"),
    files: useShortcutLabel("workbench.files"),
    git: useShortcutLabel("workbench.gitReview"),
    terminal: useShortcutLabel("workbench.terminal"),
  };
  const right = activeWorkbenchSurface(workbench.state, "right")?.kind ?? null;
  const bottom = activeWorkbenchSurface(workbench.state, "bottom")?.kind ?? null;
  const total = 1 + workbench.layout.secondary.length;
  const maximized = workbench.layout.maximized != null;

  return useMemo(() => {
    const item = (id: DesktopMenuId, label: string, hint?: string, extra?: { checked?: boolean; disabled?: boolean }): MenuEntry => ({
      kind: "item",
      id,
      label,
      hint,
      checked: extra?.checked,
      disabled: extra?.disabled,
      onPress: () => runDesktopMenuAction(id),
    });
    const dock = (kind: WorkbenchSurfaceKind, id: DesktopMenuId, label: string, hint?: string): MenuEntry =>
      item(id, label, hint, { checked: right === kind || bottom === kind });
    return [
      { kind: "heading", id: "h-presets", label: "LAYOUTS" },
      item("view:layout-focus", "Focus", undefined, { checked: total === 1 && right == null && bottom == null }),
      item("view:layout-side-by-side", "Side by side", undefined, { checked: total === 2 && right == null && bottom == null }),
      item("view:layout-grid", "Grid 2 x 2", undefined, { checked: total === 4 && right == null && bottom == null }),
      item("view:layout-review", "Review", undefined, { checked: total === 1 && right === "git" }),
      { kind: "separator", id: "s-panes" },
      { kind: "heading", id: "h-panes", label: `PANES · ${total} OF ${MAX_PANES}` },
      item("view:split-pane", "Split pane", hints.split, { disabled: total >= MAX_PANES }),
      item("view:pane-close", "Close pane", hints.close, { disabled: total <= 1 }),
      item("view:pane-maximize", maximized ? "Restore panes" : "Maximize pane", hints.maximize, { disabled: total <= 1 }),
      item("view:pane-next", "Next pane", hints.next, { disabled: total <= 1 }),
      item("view:pane-prev", "Previous pane", hints.prev, { disabled: total <= 1 }),
      { kind: "separator", id: "s-panels" },
      { kind: "heading", id: "h-panels", label: "PANELS" },
      item("view:sidebar", "Sidebar", hints.sidebar),
      dock("tasks", "view:tasks", "Tasks", hints.tasks),
      dock("agents", "view:agents", "Agents", hints.agents),
      dock("plan", "view:plan", "Plan", hints.plan),
      dock("files", "view:files", "Files", hints.files),
      dock("git", "view:git-review", "Git review", hints.git),
      dock("terminal", "view:terminal", "Terminal", hints.terminal),
      dock("preview", "view:browser-preview", "Browser preview"),
      dock("gallery", "view:gallery", "Gallery"),
      dock("actions", "view:actions", "GitHub Actions"),
      dock("notes", "view:notes", "Notes"),
      dock("usage", "view:usage", "Usage", hints.usage),
      { kind: "separator", id: "s-reset" },
      item("view:layout-reset", "Reset layout"),
    ];
  }, [bottom, hints.agents, hints.close, hints.files, hints.git, hints.maximize, hints.next, hints.plan, hints.prev, hints.sidebar, hints.split, hints.tasks, hints.terminal, hints.usage, maximized, right, total]);
}

/** Title-bar trigger. `compact` drops the label for narrow windows. */
export function LayoutControl() {
  const tokens = useTokens();
  const workbench = useWorkbench();
  const entries = useLayoutMenuEntries();
  const { width: windowWidth } = useWindowDimensions();
  const ref = useRef<View>(null);
  const [anchor, setAnchor] = useState<MenuAnchor | null>(null);
  const total = 1 + workbench.layout.secondary.length;
  const open = () =>
    ref.current?.measureInWindow((x, y, width, height) =>
      setAnchor({ top: y + height + 4, right: Math.max(8, windowWidth - x - width) }),
    );

  return (
    <>
      <Pressable
        ref={ref}
        onPress={open}
        accessibilityRole="button"
        accessibilityLabel="Layout"
        accessibilityHint="Split panes, show panels, choose a layout"
        accessibilityState={{ expanded: anchor != null }}
        style={({ pressed, hovered }: { pressed: boolean; hovered?: boolean }) => [
          styles.button,
          { borderColor: hovered || pressed || anchor ? tokens.borderStrong : tokens.border },
        ]}
      >
        <LayoutPanelLeft size={12} color={tokens.ink3} strokeWidth={1.75} />
        <Text style={[styles.label, { color: tokens.ink2 }]}>Layout</Text>
        {total > 1 ? (
          <View style={[styles.count, { backgroundColor: tokens.accent }]}>
            <Text style={[styles.countText, { color: tokens.bg0 }]}>{total}</Text>
          </View>
        ) : null}
        <ChevronDown size={11} color={tokens.ink4} strokeWidth={2} />
      </Pressable>
      <PopoverMenu anchor={anchor} entries={entries} onClose={() => setAnchor(null)} accessibilityLabel="Layout menu" />
    </>
  );
}

const styles = StyleSheet.create({
  button: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    borderWidth: 1,
    borderRadius: radii.radius4,
    paddingHorizontal: 7,
    paddingVertical: 2,
  },
  label: { fontFamily: monoFamily.regular, fontSize: 10.5 },
  count: { minWidth: 14, height: 14, borderRadius: 7, alignItems: "center", justifyContent: "center", paddingHorizontal: 3 },
  countText: { fontFamily: monoFamily.bold, fontSize: 9 },
});
