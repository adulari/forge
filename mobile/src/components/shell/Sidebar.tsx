// Machined desktop shell — the 232px expanded sidebar (D Main, docs/design/machined
// INVENTORY.md L38-74): search row (⌘P thread-search), "+ New session" (⌘N),
// project-grouped session rows (dense 28px — NOT the 56pt ds/ListRow; see INVENTORY's
// "same data model, a new dense variant for desktop" note), a FORGE ANYWHERE host
// group, and a footer (usage-pace rings + Inbox/History/Settings + the collapse
// control). Replaces `ExpandedFleetRail` (components/fleet/DesktopDrillDown.tsx).
import { router, usePathname } from "expo-router";
import { Bell, BellDot, Gauge, History, PanelLeftClose, Plus, Search, Settings2 } from "lucide-react-native";
import React, { useMemo, useRef, useState } from "react";
import { Platform, Pressable, ScrollView, StyleSheet, Text, View, type GestureResponderEvent } from "react-native";

import { useAnywhere, useAnywhereHosts } from "../../lib/anywhere/store";
import type { AnywhereHost } from "../../lib/anywhere/types";
import { hostStateText } from "../../lib/anywhere/format";
import type { SessionRow } from "../../lib/api";
import { useShortcutLabel } from "../../lib/shortcuts";
import { totalUnseen, useNoticesStore } from "../../lib/notices/store";
import { useAuth } from "../../lib/auth";
import { usePrefetchSessionHistory, useSessions } from "../../lib/queries";
import { useScopedFleet, type ScopedSessionRow } from "../../lib/useScopedFleet";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space, type StatusDotState } from "../../theme/tokens";
import { formatCost, formatCwd, formatRelativeTime, monoFamily, tabularNums, type as typeScale } from "../../theme/typography";
import { HostDot } from "../anywhere/HostDot";
import { useRelativeClock } from "../ds/relativeClock";
import { StatusDot } from "../ds/StatusDot";
import { ServerScopePills } from "../fleet/ServerScopePills";
import { usePalette } from "../overlay/CommandPalette";
import { MAX_PANES } from "../workbench/layoutModel";
import { useWorkbench, type OpenMode } from "../workbench/WorkbenchProvider";
import { PopoverMenu, type MenuAnchor, type MenuEntry } from "./PopoverMenu";
import { useDragSource } from "./sessionDrag";
import { useProviderPace, UsageRing } from "./UsageRing";

const ROW_HEIGHT = 28;

/** Groups sessions by project name, preserving first-seen order (server already
 * sorts sessions by recency, so the first group to appear is the most recently
 * active project). */
function projectNameFromCwd(cwd: string): string {
  const label = formatCwd(cwd);
  const wtIndex = label.indexOf(" · wt ");
  return (wtIndex >= 0 ? label.slice(0, wtIndex) : label).toUpperCase();
}

interface ProjectGroup {
  key: string;
  project: string;
  serverId: string;
  rows: ScopedSessionRow[];
}

/** Same-named projects on two servers are two checkouts, so the group key includes the server. */
function groupSessionsByProject(rows: ScopedSessionRow[]): ProjectGroup[] {
  const groups = new Map<string, ProjectGroup>();
  for (const item of rows) {
    const project = projectNameFromCwd(item.row.cwd);
    const key = `${item.serverId}\u0000${project}`;
    let group = groups.get(key);
    if (!group) {
      group = { key, project, serverId: item.serverId, rows: [] };
      groups.set(key, group);
    }
    group.rows.push(item);
  }
  return [...groups.values()];
}

function hostShortMeta(host: AnywhereHost): string {
  if (host.state.kind === "online") return host.state.activity === "busy" ? `busy · ${host.state.sessionCount}` : "anywhere";
  if (host.state.kind === "stale") return `stale ${formatRelativeTime(host.state.lastSeenAt)}`;
  if (host.state.kind === "offline") return "offline";
  return hostStateText(host);
}

interface DenseRowProps {
  row: SessionRow;
  serverId: string;
  selected: boolean;
  /** 1-based pane number when this session is on screen in a multi-pane layout. */
  paneNumber: number | null;
  paneFocused: boolean;
  /** How many panes are open — decides whether "open in new pane" can add one. */
  paneTotal: number;
  onOpen: (id: string, mode: OpenMode) => void;
  onClosePane: (id: string) => void;
}

function DenseSessionRow({ row, serverId, selected, paneNumber, paneFocused, paneTotal, onOpen, onClosePane }: DenseRowProps) {
  const tokens = useTokens();
  const { setActive } = useAuth();
  const prefetchHistory = usePrefetchSessionHistory();
  // Re-render on the shared clock: the list only re-renders when the fleet changes, so a
  // computed "4m" otherwise froze at whatever it read when the row last changed.
  useRelativeClock();
  const state: StatusDotState = row.waiting ? "waiting" : row.busy ? "busy" : "idle";
  const title = row.title || `session ${row.id.slice(0, 8)}`;
  const ref = useRef<View>(null);
  const [menu, setMenu] = useState<MenuAnchor | null>(null);
  // Drag the row onto a pane (or the "New pane" zone that appears while dragging).
  useDragSource(ref, { kind: "session", id: row.id });

  const open = (mode: OpenMode) => {
    if (serverId) setActive(serverId);
    onOpen(row.id, mode);
  };
  const onPress = (event: GestureResponderEvent) => {
    // Alt/Shift-click opens beside the current pane instead of replacing the focused one.
    const modifiers = event.nativeEvent as unknown as { altKey?: boolean; shiftKey?: boolean };
    open(modifiers.altKey || modifiers.shiftKey ? "new" : "focused");
  };
  const webContextMenu =
    Platform.OS === "web"
      ? ({
          onContextMenu: (e: { preventDefault: () => void; clientX: number; clientY: number }) => {
            e.preventDefault();
            setMenu({ top: e.clientY, left: e.clientX });
          },
        } as Record<string, unknown>)
      : null;
  const menuEntries: MenuEntry[] = [
    { kind: "item", id: "open", label: "Open", onPress: () => open("focused") },
    {
      kind: "item",
      id: "split",
      label: "Open in split",
      hint: paneTotal >= MAX_PANES ? "replaces last" : undefined,
      onPress: () => open("new"),
    },
    ...(paneNumber != null && paneTotal > 1
      ? ([{ kind: "item", id: "close", label: `Close pane ${paneNumber}`, onPress: () => onClosePane(row.id) }] as MenuEntry[])
      : []),
  ];

  return (
    <>
      <Pressable
        ref={ref}
        onPress={onPress}
        // Start the transcript fetch while the button is still going down: the click lands ~100 ms
        // later and the session screen then paints from cache instead of a spinner.
        onHoverIn={() => prefetchHistory(row.id)}
        onPressIn={() => prefetchHistory(row.id)}
        accessibilityRole="button"
        accessibilityLabel={`${title}, ${state}${row.waiting ? ", needs a decision" : ""}${paneNumber != null && paneTotal > 1 ? `, open in pane ${paneNumber}` : ""}`}
        accessibilityHint="Opens in the focused pane. Right-click for split options, or drag onto a pane."
        style={[
          styles.denseRow,
          (row.waiting || selected || paneNumber != null) && { backgroundColor: tokens.selection },
          paneNumber != null && paneTotal > 1 && paneFocused && { borderLeftWidth: 2, borderLeftColor: tokens.accent },
        ]}
        {...(webContextMenu as object | null)}
      >
        <StatusDot state={state} size={6} />
        <Text
          style={[typeScale.body, styles.denseTitle, { color: state === "idle" ? tokens.ink2 : tokens.ink, fontSize: 11.5 }]}
          numberOfLines={1}
        >
          {title}
        </Text>
        {row.worktree ? <Text style={[typeScale.monoMeta, { color: tokens.accent }]}>⑂</Text> : null}
        {paneNumber != null && paneTotal > 1 ? (
          <Text style={[typeScale.monoMeta, tabularNums, styles.paneBadge, { color: tokens.accent, borderColor: tokens.accent }]}>
            {paneNumber}
          </Text>
        ) : null}
        {row.waiting ? (
          <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.ink3 }]}>
            {formatRelativeTime(row.last_activity * 1000)}
          </Text>
        ) : row.busy ? (
          <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.ink2 }]}>{formatCost(row.cost_usd)}</Text>
        ) : (
          <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.ink4 }]}>
            {formatRelativeTime(row.last_activity * 1000)}
          </Text>
        )}
      </Pressable>
      <PopoverMenu anchor={menu} entries={menuEntries} onClose={() => setMenu(null)} accessibilityLabel={`${title} actions`} width={248} />
    </>
  );
}

function HostRow({ host }: { host: AnywhereHost }) {
  const tokens = useTokens();
  return (
    <Pressable
      onPress={() => router.push(`/anywhere/host/${host.id}`)}
      accessibilityRole="button"
      accessibilityLabel={`${host.name}, ${hostStateText(host)}`}
      style={styles.denseRow}
    >
      <HostDot state={host.state} size={6} />
      <Text style={[typeScale.body, styles.denseTitle, { color: tokens.ink2, fontSize: 11.5 }]} numberOfLines={1}>
        {host.name}
      </Text>
      <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]}>{hostShortMeta(host)}</Text>
    </Pressable>
  );
}

function GroupLabel({ label, server, count }: { label: string; server?: string; count: number }) {
  const tokens = useTokens();
  return (
    <View style={styles.groupLabelRow}>
      <Text style={[typeScale.section, styles.groupLabelText, { color: tokens.ink3 }]} numberOfLines={1}>
        {label}
        {server ? <Text style={{ color: tokens.ink4 }}>{` · ${server}`}</Text> : null}
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
  const usageHint = useShortcutLabel("shell.usage");
  const sidebarHint = useShortcutLabel("shell.sidebar");
  const pathname = usePathname();
  const palette = usePalette();
  const { servers } = useAuth();
  const fleet = useScopedFleet();
  const rows = fleet.rows;
  const groups = useMemo(() => groupSessionsByProject(rows), [rows]);
  const labelServers = fleet.showAll && new Set(groups.map((group) => group.serverId)).size > 1;
  const serverName = (id: string) => servers.find((server) => server.id === id)?.name;
  const noticesUnseen = totalUnseen(useNoticesStore().hosts);
  // The Inbox lists the active server's waiting sessions, so its badge counts the same set.
  const { data: activeRows } = useSessions();
  const waitingCount = useMemo(() => (activeRows ?? []).filter((r) => r.waiting).length, [activeRows]);
  const selectedSessionId = pathname.match(/^\/session\/([^/]+)/)?.[1];
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

  const { rings } = useProviderPace();

  return (
    <View style={styles.sidebar}>
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
          <Text style={[typeScale.sub, styles.emptyText, { color: tokens.ink3 }]}>No sessions — forge one above.</Text>
        ) : (
          groups.map((group) => (
            <View key={group.key}>
              <GroupLabel label={group.project} server={labelServers ? serverName(group.serverId) : undefined} count={group.rows.length} />
              {group.rows.map(({ row, serverId }) => (
                <DenseSessionRow
                  key={row.id}
                  row={row}
                  serverId={serverId}
                  selected={row.id === selectedSessionId}
                  paneNumber={paneNumbers.get(row.id) ?? null}
                  paneFocused={paneNumbers.get(row.id) === workbench.layout.focus + 1}
                  paneTotal={paneTotal}
                  onOpen={workbench.openSession}
                  onClosePane={(id) => {
                    const index = workbench.paneIds.indexOf(id);
                    if (index >= 0) workbench.closePane(index);
                  }}
                />
              ))}
            </View>
          ))
        )}

        {anywhereSignedIn && anywhereHosts.length > 0 ? (
          <View>
            <GroupLabel label="FORGE ANYWHERE" count={anywhereHosts.length} />
            {anywhereHosts.map((host) => (
              <HostRow key={host.id} host={host} />
            ))}
          </View>
        ) : null}
      </ScrollView>

      <View style={[styles.footer, { borderTopColor: tokens.border }]}>
        {rings.length > 0 ? (
          <Pressable
            onPress={onToggleDock}
            disabled={!onToggleDock}
            accessibilityRole={onToggleDock ? "button" : undefined}
            accessibilityLabel="Toggle usage dock"
            accessibilityHint={usageHint}
            style={styles.usageBlock}
          >
            <Text style={[typeScale.section, styles.usageLabel, { color: tokens.ink3 }]}>Usage</Text>
            <View style={styles.ringRow}>
              {rings.map((ring) => (
                // A bare ring at a low percent reads as a loading spinner, so it carries its number.
                <View key={ring.provider} style={styles.ringItem}>
                  <UsageRing pct={ring.pct} accessibilityLabel={`${ring.provider} usage ${ring.pct}%`} />
                  <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.ink3 }]}>
                    {planName(ring.provider)} {ring.pct}%
                  </Text>
                </View>
              ))}
            </View>
          </Pressable>
        ) : null}

        <View style={styles.footerIcons}>
          <Pressable
            onPress={() => router.push("/inbox")}
            accessibilityRole="button"
            accessibilityState={{ selected: pathname === "/inbox" }}
            accessibilityLabel={waitingCount > 0 ? `Inbox, ${waitingCount} needs you` : "Inbox"}
            style={[styles.footerIconButton, pathname === "/inbox" && { backgroundColor: tokens.selection }]}
          >
            <BellDot size={15} strokeWidth={1.75} color={tokens.ink2} />
            {waitingCount > 0 ? <View style={[styles.footerBadge, { backgroundColor: tokens.danger, borderColor: tokens.bg1 }]} /> : null}
          </Pressable>
          <Pressable
            onPress={() => router.push("/notices")}
            accessibilityRole="button"
            accessibilityState={{ selected: pathname === "/notices" }}
            accessibilityLabel={noticesUnseen > 0 ? `Notices, ${noticesUnseen} new` : "Notices"}
            style={[styles.footerIconButton, pathname === "/notices" && { backgroundColor: tokens.selection }]}
          >
            <Bell size={15} strokeWidth={1.75} color={tokens.ink2} />
            {noticesUnseen > 0 ? <View style={[styles.footerBadge, { backgroundColor: tokens.accent, borderColor: tokens.bg1 }]} /> : null}
          </Pressable>
          <Pressable
            onPress={() => router.push("/history")}
            accessibilityRole="button"
            accessibilityState={{ selected: pathname === "/history" }}
            accessibilityLabel="History"
            style={[styles.footerIconButton, pathname === "/history" && { backgroundColor: tokens.selection }]}
          >
            <History size={15} strokeWidth={1.75} color={tokens.ink2} />
          </Pressable>
          <View style={styles.footerSpacer} />
          {/* Always present: the ring row above only renders once a subscription window has
              been observed, and without this the dock had no visible way to open. */}
          {onToggleDock ? (
            <Pressable
              onPress={onToggleDock}
              accessibilityRole="button"
              accessibilityLabel={dockOpen ? "Hide usage panel" : "Show usage panel"}
              accessibilityHint={usageHint}
              accessibilityState={{ selected: dockOpen }}
              style={[styles.footerIconButton, dockOpen && { backgroundColor: tokens.bg3 }]}
            >
              <Gauge size={15} strokeWidth={1.75} color={dockOpen ? tokens.ink : tokens.ink2} />
            </Pressable>
          ) : null}
          <Pressable
            onPress={() => router.push("/settings")}
            accessibilityRole="button"
            accessibilityState={{ selected: pathname === "/settings" }}
            accessibilityLabel="Settings"
            style={[styles.footerIconButton, pathname === "/settings" && { backgroundColor: tokens.selection }]}
          >
            <Settings2 size={15} strokeWidth={1.75} color={tokens.ink2} />
          </Pressable>
          <Pressable
            onPress={onCollapse}
            accessibilityRole="button"
            accessibilityLabel="Collapse sidebar"
            accessibilityHint={sidebarHint}
            style={styles.footerIconButton}
          >
            <PanelLeftClose size={15} strokeWidth={1.75} color={tokens.ink2} />
          </Pressable>
        </View>
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  sidebar: { flex: 1, paddingHorizontal: space.space8, paddingTop: space.space8 },
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
  denseRow: {
    height: ROW_HEIGHT,
    borderRadius: radii.radius4,
    flexDirection: "row",
    alignItems: "center",
    gap: space.space8,
    paddingHorizontal: space.space8,
  },
  denseTitle: { flex: 1 },
  paneBadge: { borderWidth: 1, borderRadius: 3, paddingHorizontal: 3, minWidth: 14, textAlign: "center" },
  footer: { borderTopWidth: StyleSheet.hairlineWidth, paddingTop: space.space8, paddingBottom: space.space4 },
  usageBlock: { paddingHorizontal: space.space4, marginBottom: space.space8 },
  usageLabel: { fontFamily: monoFamily.regular, marginBottom: space.space8 },
  ringRow: { flexDirection: "row", gap: space.space12, alignItems: "center" },
  ringItem: { flexDirection: "row", gap: space.space4, alignItems: "center" },
  footerIcons: { flexDirection: "row", alignItems: "center", gap: space.space4, paddingHorizontal: space.space2 },
  footerIconButton: { width: 30, height: 30, alignItems: "center", justifyContent: "center", borderRadius: radii.radius8 },
  footerBadge: { position: "absolute", top: 4, right: 4, width: 6, height: 6, borderRadius: 3, borderWidth: 1 },
  footerSpacer: { flex: 1 },
});

function planName(provider: string): string {
  if (provider.startsWith("claude")) return "Claude";
  if (provider.startsWith("codex")) return "Codex";
  return provider;
}
