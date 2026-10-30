// Machined desktop shell — the expanded sidebar (D Main, docs/design/machined
// INVENTORY.md L38-74), 232px by default and drag-resizable between 200 and 360: workspace
// switcher + sort toggle, search row (⌘P thread-search), "+ New session" (⌘N), foldable
// project groups of dense 28px session rows (NOT the 56pt ds/ListRow; see INVENTORY's "same
// data model, a new dense variant for desktop" note) with forks nested under their parent, a
// FORGE ANYWHERE host group, and a footer. Replaces `ExpandedFleetRail`
// (components/fleet/DesktopDrillDown.tsx).
import { router, usePathname } from "expo-router";
import { ArrowUpDown, Plus, Search } from "lucide-react-native";
import React, { useMemo, useState } from "react";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";

import { useAnywhere, useAnywhereHosts } from "../../lib/anywhere/store";
import type { AnywhereHost } from "../../lib/anywhere/types";
import { hostStateText } from "../../lib/anywhere/format";
import { ServerScope, useAuth } from "../../lib/auth";
import { useShortcutLabel } from "../../lib/shortcuts";
import { totalUnseen, useNoticesStore } from "../../lib/notices/store";
import { useSessionActivationOverride } from "../../lib/sessionPrewarm";
import { useSessions } from "../../lib/queries";
import {
  activeWorkspaceProjects,
  clampSidebarWidth,
  saveShellPrefs,
  setSidebarSort,
  setSidebarWidth,
  SIDEBAR_DEFAULT_WIDTH,
  toggleGroupCollapsed,
  useShellPrefs,
} from "../../lib/shellPrefs";
import { buildProjectGroups, filterByWorkspace, projectName, sortSessions, splitDone, walkNodes } from "../../lib/sidebarModel";
import { useScopedFleet } from "../../lib/useScopedFleet";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { formatRelativeTime, monoFamily, type as typeScale } from "../../theme/typography";
import { HostDot } from "../anywhere/HostDot";
import { ServerScopePills } from "../fleet/ServerScopePills";
import { usePalette } from "../overlay/CommandPalette";
import { useWorkbench } from "../workbench/WorkbenchProvider";
import { ResizeHandle } from "./ResizeHandle";
import { ROW_HEIGHT, SidebarSessionRow } from "./SidebarSessionRow";
import { SidebarFooter } from "./SidebarFooter";
import { SidebarProjectHeader } from "./SidebarProjectHeader";
import { WorkspaceSwitcher } from "./WorkspaceSwitcher";

function hostShortMeta(host: AnywhereHost): string {
  if (host.state.kind === "online") return host.state.activity === "busy" ? `busy · ${host.state.sessionCount}` : "anywhere";
  if (host.state.kind === "stale") return `stale ${formatRelativeTime(host.state.lastSeenAt)}`;
  if (host.state.kind === "offline") return "offline";
  return hostStateText(host);
}

function HostRow({ host }: { host: AnywhereHost }) {
  const tokens = useTokens();
  return (
    <Pressable
      onPress={() => router.push(`/anywhere/host/${host.id}`)}
      accessibilityRole="button"
      accessibilityLabel={`${host.name}, ${hostStateText(host)}`}
      style={styles.hostRow}
    >
      <HostDot state={host.state} size={6} />
      <Text style={[typeScale.body, styles.hostTitle, { color: tokens.ink2, fontSize: 11.5 }]} numberOfLines={1}>
        {host.name}
      </Text>
      <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]}>{hostShortMeta(host)}</Text>
    </Pressable>
  );
}

function GroupLabel({ label, count }: { label: string; count: number }) {
  const tokens = useTokens();
  return (
    <View style={styles.groupLabelRow}>
      <Text style={[typeScale.section, styles.groupLabelText, { color: tokens.ink3 }]} numberOfLines={1}>
        {label}
      </Text>
      <Text style={[typeScale.monoMeta, { color: tokens.ink4 }]}>{count}</Text>
    </View>
  );
}

export interface SidebarProps {
  onCollapse: () => void;
  /** Present only at the expanded breakpoint on desktop/web — lets the footer's
   * usage-ring row double as the ⌘U dock-toggle affordance (design calls for
   * "⌘U + a top-bar/icon-rail affordance", this rail's half of that). */
  onToggleDock?: () => void;
  dockOpen?: boolean;
}

export function Sidebar({ onCollapse, onToggleDock, dockOpen = false }: SidebarProps) {
  const tokens = useTokens();
  const searchHint = useShortcutLabel("session.search");
  const newSessionHint = useShortcutLabel("session.new");
  const pathname = usePathname();
  const palette = usePalette();
  const { servers } = useAuth();
  const prefs = useShellPrefs();
  const fleet = useScopedFleet();
  const workspaceProjects = activeWorkspaceProjects(prefs);
  const rows = useMemo(() => filterByWorkspace(fleet.rows, workspaceProjects), [fleet.rows, workspaceProjects]);
  const split = useMemo(() => splitDone(rows), [rows]);
  const groups = useMemo(() => buildProjectGroups(split.active, prefs.sort), [split.active, prefs.sort]);
  const doneRows = useMemo(() => sortSessions(split.done, prefs.sort), [split.done, prefs.sort]);
  const [doneOpen, setDoneOpen] = useState(false);
  const allProjectNames = useMemo(
    () => [...new Set(fleet.rows.map((entry) => projectName(entry.row.cwd)))].sort(),
    [fleet.rows],
  );
  const labelServers = fleet.showAll && new Set(groups.map((group) => group.serverId)).size > 1;
  const serverName = (id: string) => servers.find((server) => server.id === id)?.name;
  const noticesUnseen = totalUnseen(useNoticesStore().hosts);
  // The Inbox lists the active server's waiting sessions, so its badge counts the same set.
  const { data: activeRows } = useSessions();
  const waitingCount = useMemo(
    () => (activeRows ?? []).filter((r) => r.waiting && (!workspaceProjects || workspaceProjects.has(projectName(r.cwd)))).length,
    [activeRows, workspaceProjects],
  );
  const activationOverride = useSessionActivationOverride();
  const selectedSessionId = activationOverride ?? pathname.match(/^\/session\/([^/]+)/)?.[1];
  const workbench = useWorkbench();
  const paneTotal = workbench.paneIds.length;
  const paneNumbers = useMemo(() => {
    const map = new Map<string, number>();
    workbench.paneIds.forEach((id, index) => {
      if (id) map.set(id, index + 1);
    });
    return map;
  }, [workbench.paneIds]);

  const { signedIn: anywhereSignedIn } = useAnywhere();
  const { hosts: anywhereHosts } = useAnywhereHosts();

  const dragStart = React.useRef(prefs.sidebarWidth);
  const sortLabel = prefs.sort === "updated" ? "last updated" : "newest first";

  return (
    <View style={styles.sidebar}>
      <View style={styles.headerRow}>
        <WorkspaceSwitcher projectNames={allProjectNames} />
        <Pressable
          onPress={() => setSidebarSort(prefs.sort === "updated" ? "created" : "updated")}
          accessibilityRole="button"
          accessibilityLabel={`Sort sessions: ${sortLabel}`}
          accessibilityHint="Switches between last updated and newest first"
          style={({ hovered }: { pressed: boolean; hovered?: boolean }) => [styles.sortButton, hovered && { backgroundColor: tokens.bg3 }]}
        >
          <ArrowUpDown size={12} strokeWidth={1.75} color={tokens.ink3} />
          <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]}>{prefs.sort === "updated" ? "Recent" : "Newest"}</Text>
        </Pressable>
      </View>

      <Pressable
        onPress={() => palette.open("search")}
        accessibilityRole="button"
        accessibilityLabel="Search sessions"
        style={[styles.searchRow, { backgroundColor: tokens.bg3, borderColor: tokens.border }]}
      >
        <Search size={12} strokeWidth={1.75} color={tokens.ink3} />
        <Text style={[typeScale.sub, styles.searchHint, { color: tokens.ink3 }]}>Search sessions…</Text>
        <Text style={[typeScale.monoMeta, { color: tokens.ink4 }]}>{searchHint}</Text>
      </Pressable>

      <Pressable
        onPress={() => router.push("/new-session")}
        accessibilityRole="button"
        accessibilityLabel="Forge a new session"
        style={[styles.newSessionRow, { borderColor: tokens.borderStrong }]}
      >
        <Plus size={13} strokeWidth={2} color={tokens.ink} />
        <Text style={[typeScale.bodyBold, { color: tokens.ink }]}>New session</Text>
        <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]}>{newSessionHint}</Text>
      </Pressable>

      <ServerScopePills layout="wrap" />

      <ScrollView style={styles.list} contentContainerStyle={styles.listContent}>
        {fleet.isLoading && rows.length === 0 ? (
          <Text style={[typeScale.sub, styles.emptyText, { color: tokens.ink3 }]}>Connecting…</Text>
        ) : rows.length === 0 ? (
          <Text style={[typeScale.sub, styles.emptyText, { color: tokens.ink3 }]}>
            {fleet.rows.length > 0 ? "Nothing in this workspace yet. Pick its projects from the switcher above." : "No sessions — forge one above."}
          </Text>
        ) : (
          groups.map((group) => {
            const collapsed = prefs.collapsedGroups.includes(group.key);
            const members = collapsed ? [] : [...walkNodes(group.nodes)];
            const perDirectory = new Map<string, number>();
            for (const { node } of members) {
              const dir = `${node.entry.serverId}\u0000${node.entry.row.cwd}`;
              perDirectory.set(dir, (perDirectory.get(dir) ?? 0) + 1);
            }
            return (
              <View key={group.key}>
                <SidebarProjectHeader
                  group={group}
                  collapsed={collapsed}
                  serverName={labelServers ? serverName(group.serverId) : undefined}
                  workspaces={prefs.workspaces}
                  onToggle={() => toggleGroupCollapsed(group.key)}
                />
                {members.map(({ node, depth }) => (
                      <ServerScope key={node.entry.row.id} serverId={node.entry.serverId}>
                        <SidebarSessionRow
                          row={node.entry.row}
                          serverId={node.entry.serverId}
                          depth={depth}
                          showDiff={perDirectory.get(`${node.entry.serverId}\u0000${node.entry.row.cwd}`) === 1}
                          selected={node.entry.row.id === selectedSessionId}
                          paneNumber={paneNumbers.get(node.entry.row.id) ?? null}
                          paneFocused={paneNumbers.get(node.entry.row.id) === workbench.layout.focus + 1}
                          paneTotal={paneTotal}
                          onOpen={workbench.openSession}
                          onClosePane={(id) => {
                            const index = workbench.paneIds.indexOf(id);
                            if (index >= 0) workbench.closePane(index);
                          }}
                        />
                      </ServerScope>
                ))}
              </View>
            );
          })
        )}

        {doneRows.length > 0 ? (
          <View>
            <Pressable
              onPress={() => setDoneOpen((open) => !open)}
              accessibilityRole="button"
              accessibilityState={{ expanded: doneOpen }}
              accessibilityLabel={`Done, ${doneRows.length} sessions, ${doneOpen ? "collapse" : "expand"}`}
            >
              <GroupLabel label={`${doneOpen ? "▾" : "▸"} DONE`} count={doneRows.length} />
            </Pressable>
            {doneOpen
              ? doneRows.map((entry) => (
                  <ServerScope key={entry.row.id} serverId={entry.serverId}>
                    <SidebarSessionRow
                      row={entry.row}
                      serverId={entry.serverId}
                      depth={0}
                      showDiff={false}
                      selected={entry.row.id === selectedSessionId}
                      paneNumber={paneNumbers.get(entry.row.id) ?? null}
                      paneFocused={paneNumbers.get(entry.row.id) === workbench.layout.focus + 1}
                      paneTotal={paneTotal}
                      onOpen={workbench.openSession}
                      onClosePane={(id) => {
                        const index = workbench.paneIds.indexOf(id);
                        if (index >= 0) workbench.closePane(index);
                      }}
                    />
                  </ServerScope>
                ))
              : null}
          </View>
        ) : null}

        {anywhereSignedIn && anywhereHosts.length > 0 ? (
          <View>
            <GroupLabel label="FORGE ANYWHERE" count={anywhereHosts.length} />
            {anywhereHosts.map((host) => (
              <HostRow key={host.id} host={host} />
            ))}
          </View>
        ) : null}
      </ScrollView>

      <SidebarFooter
        onCollapse={onCollapse}
        onToggleDock={onToggleDock}
        dockOpen={dockOpen}
        waitingCount={waitingCount}
        noticesUnseen={noticesUnseen}
      />

      <ResizeHandle
        orientation="vertical"
        overlay="end"
        label="Resize sidebar"
        onStart={() => {
          dragStart.current = prefs.sidebarWidth;
        }}
        onDrag={(delta) => setSidebarWidth(dragStart.current + delta, false)}
        onEnd={saveShellPrefs}
        onReset={() => setSidebarWidth(SIDEBAR_DEFAULT_WIDTH)}
        onNudge={(delta) => setSidebarWidth(clampSidebarWidth(prefs.sidebarWidth + delta))}
      />
    </View>
  );
}

const styles = StyleSheet.create({
  sidebar: { flex: 1, paddingHorizontal: space.space8, paddingTop: space.space8 },
  headerRow: { flexDirection: "row", alignItems: "center", gap: space.space4, marginBottom: space.space8 },
  sortButton: {
    height: 26,
    flexDirection: "row",
    alignItems: "center",
    gap: 4,
    paddingHorizontal: space.space8,
    borderRadius: radii.radius4,
  },
  searchRow: {
    height: ROW_HEIGHT - 1,
    borderWidth: 1,
    borderRadius: radii.radius4,
    flexDirection: "row",
    alignItems: "center",
    gap: space.space8,
    paddingHorizontal: space.space8,
  },
  searchHint: { flex: 1 },
  newSessionRow: {
    height: ROW_HEIGHT - 1,
    marginTop: space.space8,
    borderWidth: 1,
    borderRadius: radii.radius4,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: space.space8,
  },
  list: { flex: 1, marginTop: space.space8 },
  listContent: { paddingBottom: space.space12 },
  emptyText: { paddingHorizontal: space.space8, paddingTop: space.space16, textAlign: "center" },
  groupLabelRow: { flexDirection: "row", alignItems: "center", marginTop: space.space12, marginBottom: 4, paddingHorizontal: space.space4 },
  groupLabelText: { flex: 1, fontFamily: monoFamily.regular },
  hostRow: {
    height: ROW_HEIGHT,
    borderRadius: radii.radius4,
    flexDirection: "row",
    alignItems: "center",
    gap: space.space8,
    paddingHorizontal: space.space8,
  },
  hostTitle: { flex: 1 },
});
