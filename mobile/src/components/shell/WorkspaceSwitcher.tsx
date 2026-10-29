// The sidebar header's workspace switcher. A workspace is a named set of projects; picking one
// narrows the sidebar, the Inbox and Schedules to those projects, and "All projects" lifts it.
// Projects join a workspace from their own menu in the sidebar.
import { ChevronDown, Layers } from "lucide-react-native";
import React, { useRef, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { createWorkspace, deleteWorkspace, setActiveWorkspace, toggleProjectInWorkspace, useShellPrefs } from "../../lib/shellPrefs";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { Button } from "../ds/Button";
import { Input } from "../ds/Input";
import { Sheet } from "../ds/Sheet";
import { PopoverMenu, type MenuAnchor, type MenuEntry } from "./PopoverMenu";

export function WorkspaceSwitcher({ projectNames }: { projectNames: readonly string[] }) {
  const tokens = useTokens();
  const prefs = useShellPrefs();
  const ref = useRef<View>(null);
  const [anchor, setAnchor] = useState<MenuAnchor | null>(null);
  const [creating, setCreating] = useState(false);
  const [name, setName] = useState("");
  const active = prefs.workspaces.find((workspace) => workspace.id === prefs.activeWorkspaceId) ?? null;

  const openMenu = () => {
    ref.current?.measureInWindow((x, y, _width, height) => setAnchor({ top: y + height + 4, left: x }));
  };

  const entries: MenuEntry[] = [
    { kind: "item", id: "all", label: "All projects", checked: active == null, onPress: () => setActiveWorkspace(null) },
    ...prefs.workspaces.map(
      (workspace): MenuEntry => ({
        kind: "item",
        id: workspace.id,
        label: workspace.name,
        hint: `${workspace.projects.length}`,
        checked: workspace.id === active?.id,
        onPress: () => setActiveWorkspace(workspace.id),
      }),
    ),
    ...(active && projectNames.length > 0
      ? ([
          { kind: "separator", id: "sep-projects" },
          { kind: "heading", id: "projects", label: `PROJECTS IN ${active.name.toUpperCase()}` },
          ...projectNames.map(
            (project): MenuEntry => ({
              kind: "item",
              id: `project-${project}`,
              label: project,
              checked: active.projects.includes(project),
              onPress: () => toggleProjectInWorkspace(active.id, project),
            }),
          ),
        ] as MenuEntry[])
      : []),
    { kind: "separator", id: "sep" },
    { kind: "item", id: "new", label: "New workspace…", onPress: () => setCreating(true) },
    ...(active
      ? ([{ kind: "item", id: "delete", label: `Delete "${active.name}"`, onPress: () => deleteWorkspace(active.id) }] as MenuEntry[])
      : []),
  ];

  const submit = () => {
    if (createWorkspace(name)) {
      setName("");
      setCreating(false);
    }
  };

  return (
    <>
      <Pressable
        ref={ref}
        onPress={openMenu}
        accessibilityRole="button"
        accessibilityLabel={`Workspace: ${active?.name ?? "All projects"}`}
        accessibilityHint="Choose which projects the sidebar, Inbox and Schedules show"
        style={({ hovered }: { pressed: boolean; hovered?: boolean }) => [styles.button, hovered && { backgroundColor: tokens.bg3 }]}
      >
        <Layers size={12} strokeWidth={1.75} color={active ? tokens.accent : tokens.ink3} />
        <Text style={[typeScale.sub, styles.name, { color: active ? tokens.ink : tokens.ink2 }]} numberOfLines={1}>
          {active?.name ?? "All projects"}
        </Text>
        <ChevronDown size={11} strokeWidth={2} color={tokens.ink3} />
      </Pressable>
      <PopoverMenu anchor={anchor} entries={entries} onClose={() => setAnchor(null)} accessibilityLabel="Workspaces" width={220} />
      <Sheet visible={creating} onClose={() => setCreating(false)} accessibilityLabel="New workspace">
        <View style={styles.sheet}>
          <Text style={[typeScale.headingBold, { color: tokens.ink }]}>New workspace</Text>
          <Text style={[typeScale.sub, { color: tokens.ink3 }]}>
            A workspace groups projects. Pick its projects from this menu, or from each project&apos;s menu in the sidebar.
          </Text>
          <Input value={name} onChangeText={setName} placeholder="Work" autoFocus onSubmitEditing={submit} accessibilityLabel="Workspace name" />
          <Button label="Create workspace" onPress={submit} disabled={name.trim().length === 0} />
        </View>
      </Sheet>
    </>
  );
}

const styles = StyleSheet.create({
  button: {
    flex: 1,
    height: 26,
    flexDirection: "row",
    alignItems: "center",
    gap: space.space8,
    paddingHorizontal: space.space8,
    borderRadius: radii.radius4,
  },
  name: { flex: 1 },
  sheet: { gap: space.space12, paddingHorizontal: space.space4, paddingBottom: space.space16 },
});
