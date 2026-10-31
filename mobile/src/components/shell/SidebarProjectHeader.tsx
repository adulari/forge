// The label row above each project's sessions in the desktop sidebar: a fold chevron, the
// project's accent swatch and name, its change total (+N lines across its checkouts), the session
// count, and, on hover or keyboard focus, a "New thread" button and the project menu.
import { router } from "expo-router";
import { ChevronDown, ChevronRight, MoreHorizontal, Plus } from "lucide-react-native";
import React, { useRef, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { useAuth } from "../../lib/auth";
import { diffLabel, projectAccent, type ProjectGroup } from "../../lib/sidebarModel";
import { toggleProjectInWorkspace, type Workspace } from "../../lib/shellPrefs";
import { useTheme } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { monoFamily, tabularNums, type as typeScale } from "../../theme/typography";
import { useActionMenuEntries } from "../ports/useActionMenuEntries";
import { PopoverMenu, type MenuAnchor, type MenuEntry } from "./PopoverMenu";

const MENU_WIDTH = 232;

export interface SidebarProjectHeaderProps {
  group: ProjectGroup;
  collapsed: boolean;
  serverName?: string;
  workspaces: readonly Workspace[];
  onToggle: () => void;
}

export function SidebarProjectHeader({ group, collapsed, serverName, workspaces, onToggle }: SidebarProjectHeaderProps) {
  const { scheme, tokens } = useTheme();
  const { setActive, activeServerId } = useAuth();
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);
  const [menu, setMenu] = useState<MenuAnchor | null>(null);
  const moreRef = useRef<View>(null);
  const actionsVisible = hovered || focused || menu != null;
  const label = group.name.toUpperCase();
  const diff = diffLabel(group.added, group.removed);

  const newThread = () => {
    if (group.serverId) setActive(group.serverId);
    router.push({ pathname: "/new-session", params: { cwd: group.root } });
  };

  // Actions are the project's, so any of its sessions answers; the most recent one is first.
  const actionSession = group.serverId === activeServerId ? (group.nodes[0]?.entry.row.id ?? null) : null;
  const actionEntries = useActionMenuEntries(actionSession, menu != null);
  const entries: MenuEntry[] = [
    { kind: "item", id: "new", label: "New thread here", onPress: newThread },
    { kind: "item", id: "fold", label: collapsed ? "Expand" : "Collapse", onPress: onToggle },
    { kind: "separator", id: "sep" },
    { kind: "heading", id: "workspaces", label: "WORKSPACES" },
    ...(workspaces.length === 0
      ? ([{ kind: "item", id: "none", label: "No workspaces yet", hint: "create one above", disabled: true, onPress: () => undefined }] as MenuEntry[])
      : workspaces.map(
          (workspace): MenuEntry => ({
            kind: "item",
            id: `ws-${workspace.id}`,
            label: workspace.name,
            checked: workspace.projects.includes(group.name),
            onPress: () => toggleProjectInWorkspace(workspace.id, group.name),
          }),
        )),
    ...actionEntries,
  ];

  return (
    <View
      style={styles.wrap}
      {...({ onMouseEnter: () => setHovered(true), onMouseLeave: () => setHovered(false) } as object)}
    >
      <Pressable
        onPress={onToggle}
        onFocus={() => setFocused(true)}
        onBlur={() => setFocused(false)}
        accessibilityRole="button"
        accessibilityState={{ expanded: !collapsed }}
        accessibilityLabel={`${label}, ${group.sessionCount} ${group.sessionCount === 1 ? "session" : "sessions"}${diff ? `, ${group.added} lines added${group.removed ? `, ${group.removed} removed` : ""}` : ""}`}
        accessibilityHint={collapsed ? "Expand the group" : "Collapse the group"}
        style={styles.toggle}
        {...(typeof document !== "undefined"
          ? ({
              onContextMenu: (e: { preventDefault: () => void; clientX: number; clientY: number }) => {
                e.preventDefault();
                setMenu({ top: e.clientY, left: e.clientX });
              },
            } as object)
          : null)}
      >
        {collapsed ? <ChevronRight size={11} strokeWidth={2} color={tokens.ink3} /> : <ChevronDown size={11} strokeWidth={2} color={tokens.ink3} />}
        <View style={[styles.swatch, { backgroundColor: projectAccent(group.name, scheme) }]} />
        <Text style={[typeScale.section, styles.label, { color: tokens.ink3 }]} numberOfLines={1}>
          {label}
          {serverName ? <Text style={{ color: tokens.ink4 }}>{` · ${serverName}`}</Text> : null}
        </Text>
        {diff ? (
          <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.success }]} numberOfLines={1}>
            +{group.added}
          </Text>
        ) : null}
        {collapsed && (group.waitingCount > 0 || group.failedCount > 0 || group.busyCount > 0) ? (
          <View
            accessibilityRole="image"
            accessibilityLabel={
              group.waitingCount > 0
                ? `${group.waitingCount} needs you`
                : group.failedCount > 0
                  ? `${group.failedCount} last turn failed`
                  : `${group.busyCount} working`
            }
            style={[styles.attention, { backgroundColor: group.waitingCount > 0 || group.failedCount > 0 ? tokens.danger : tokens.accent }]}
          />
        ) : null}
        <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.ink4 }]}>{group.sessionCount}</Text>
      </Pressable>
      <View pointerEvents={actionsVisible ? "auto" : "none"} style={[styles.actions, { backgroundColor: tokens.bg1 }, !actionsVisible && styles.actionsHidden]}>
        <Pressable
          onPress={newThread}
          onFocus={() => setFocused(true)}
          onBlur={() => setFocused(false)}
          accessibilityRole="button"
          accessibilityLabel={`New thread in ${label}`}
          style={({ hovered: over }: { pressed: boolean; hovered?: boolean }) => [styles.actionButton, over && { backgroundColor: tokens.bg3 }]}
        >
          <Plus size={12} strokeWidth={2} color={tokens.ink2} />
        </Pressable>
        <Pressable
          ref={moreRef}
          onPress={() => moreRef.current?.measureInWindow((x, y, width, height) => setMenu({ top: y + height + 4, left: Math.max(8, x + width - MENU_WIDTH) }))}
          onFocus={() => setFocused(true)}
          onBlur={() => setFocused(false)}
          accessibilityRole="button"
          accessibilityLabel={`${label} options`}
          style={({ hovered: over }: { pressed: boolean; hovered?: boolean }) => [styles.actionButton, over && { backgroundColor: tokens.bg3 }]}
        >
          <MoreHorizontal size={12} strokeWidth={2} color={tokens.ink2} />
        </Pressable>
      </View>
      <PopoverMenu anchor={menu} entries={entries} onClose={() => setMenu(null)} accessibilityLabel={`${label} options`} width={MENU_WIDTH} />
    </View>
  );
}

const styles = StyleSheet.create({
  wrap: { marginTop: space.space8, minHeight: 24, justifyContent: "center" },
  toggle: { flexDirection: "row", alignItems: "center", gap: 6, paddingHorizontal: space.space4, height: 24 },
  attention: { width: 6, height: 6, borderRadius: 3 },
  swatch: { width: 7, height: 7, borderRadius: 2 },
  label: { flex: 1, fontFamily: monoFamily.regular },
  actions: { position: "absolute", right: 0, top: 2, bottom: 2, paddingLeft: 4, flexDirection: "row", alignItems: "center", gap: 2 },
  actionsHidden: { opacity: 0 },
  actionButton: { width: 20, height: 20, borderRadius: radii.radius4, alignItems: "center", justifyContent: "center" },
});
