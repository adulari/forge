// Machined desktop shell — the 232px expanded sidebar (D Main, docs/design/machined
// INVENTORY.md L38-74): search row (⌘P thread-search), "+ New session" (⌘N),
// project-grouped session rows (dense 28px — NOT the 56pt ds/ListRow; see INVENTORY's
// "same data model, a new dense variant for desktop" note), a FORGE ANYWHERE host
// group, and a footer (usage-pace rings + Inbox/History/Settings + the collapse
// control). Replaces `ExpandedFleetRail` (components/fleet/DesktopDrillDown.tsx).
import { router, usePathname } from "expo-router";
import { BellDot, History, PanelLeftClose, Plus, Search, Settings2 } from "lucide-react-native";
import React, { useMemo } from "react";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";

import { useAnywhere, useAnywhereHosts } from "../../lib/anywhere/store";
import type { AnywhereHost } from "../../lib/anywhere/types";
import { hostStateText } from "../../lib/anywhere/format";
import type { SessionRow } from "../../lib/api";
import { modKey } from "../../lib/platform";
import { useSessions } from "../../lib/queries";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space, type StatusDotState } from "../../theme/tokens";
import { formatCost, formatCwd, formatRelativeTime, monoFamily, tabularNums, type as typeScale } from "../../theme/typography";
import { HostDot } from "../anywhere/HostDot";
import { StatusDot } from "../ds/StatusDot";
import { usePalette } from "../overlay/CommandPalette";
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

function groupSessionsByProject(rows: SessionRow[]): { project: string; rows: SessionRow[] }[] {
  const order: string[] = [];
  const map = new Map<string, SessionRow[]>();
  for (const row of rows) {
    const project = projectNameFromCwd(row.cwd);
    if (!map.has(project)) {
      map.set(project, []);
      order.push(project);
    }
    map.get(project)!.push(row);
  }
  return order.map((project) => ({ project, rows: map.get(project)! }));
}

function hostShortMeta(host: AnywhereHost): string {
  if (host.state.kind === "online") return host.state.activity === "busy" ? `busy · ${host.state.sessionCount}` : "anywhere";
  if (host.state.kind === "stale") return `stale ${formatRelativeTime(host.state.lastSeenAt)}`;
  if (host.state.kind === "offline") return "offline";
  return hostStateText(host);
}

function DenseSessionRow({ row, selected }: { row: SessionRow; selected: boolean }) {
  const tokens = useTokens();
  const state: StatusDotState = row.waiting ? "waiting" : row.busy ? "busy" : "idle";
  const title = row.title || `session ${row.id.slice(0, 8)}`;

  return (
    <Pressable
      onPress={() => router.push(`/session/${row.id}`)}
      accessibilityRole="button"
      accessibilityLabel={`${title}, ${state}${row.waiting ? ", needs a decision" : ""}`}
      style={[styles.denseRow, (row.waiting || selected) && { backgroundColor: tokens.selection }]}
    >
      <StatusDot state={state} size={6} />
      <Text
        style={[typeScale.body, styles.denseTitle, { color: state === "idle" ? tokens.ink2 : tokens.ink, fontSize: 11.5 }]}
        numberOfLines={1}
      >
        {title}
      </Text>
      {row.worktree ? <Text style={[typeScale.monoMeta, { color: tokens.accent }]}>⑂</Text> : null}
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
}

export function Sidebar({ onCollapse, onToggleDock }: SidebarProps) {
  const tokens = useTokens();
  const pathname = usePathname();
  const palette = usePalette();
  const sessionsQuery = useSessions();
  const rows = useMemo(() => sessionsQuery.data ?? [], [sessionsQuery.data]);
  const groups = useMemo(() => groupSessionsByProject(rows), [rows]);
  const waitingCount = useMemo(() => rows.filter((r) => r.waiting).length, [rows]);
  const selectedSessionId = pathname.match(/^\/session\/([^/]+)/)?.[1];

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
        <Text style={[typeScale.monoMeta, { color: tokens.ink4 }]}>{modKey}P</Text>
      </Pressable>

      <Pressable
        onPress={() => router.push("/new-session")}
        accessibilityRole="button"
        accessibilityLabel="Forge a new session"
        style={[styles.newSessionRow, { borderColor: tokens.borderStrong }]}
      >
        <Plus size={13} strokeWidth={2} color={tokens.ink} />
        <Text style={[typeScale.bodyBold, { color: tokens.ink }]}>New session</Text>
        <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]}>{modKey}N</Text>
      </Pressable>

      <ScrollView style={styles.list} contentContainerStyle={styles.listContent}>
        {sessionsQuery.isLoading && rows.length === 0 ? (
          <Text style={[typeScale.sub, styles.emptyText, { color: tokens.ink3 }]}>Connecting…</Text>
        ) : rows.length === 0 ? (
          <Text style={[typeScale.sub, styles.emptyText, { color: tokens.ink3 }]}>No sessions — forge one above.</Text>
        ) : (
          groups.map(({ project, rows: groupRows }) => (
            <View key={project}>
              <GroupLabel label={project} count={groupRows.length} />
              {groupRows.map((row) => (
                <DenseSessionRow key={row.id} row={row} selected={row.id === selectedSessionId} />
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
            accessibilityHint="Command U"
            style={styles.usageBlock}
          >
            <Text style={[typeScale.section, styles.usageLabel, { color: tokens.ink3 }]}>Usage</Text>
            <View style={styles.ringRow}>
              {rings.map((ring) => (
                <UsageRing key={ring.provider} pct={ring.pct} accessibilityLabel={`${ring.provider} usage ${ring.pct}%`} />
              ))}
            </View>
          </Pressable>
        ) : null}

        <View style={styles.footerIcons}>
          <Pressable
            onPress={() => router.push("/inbox")}
            accessibilityRole="button"
            accessibilityLabel={waitingCount > 0 ? `Inbox, ${waitingCount} needs you` : "Inbox"}
            style={styles.footerIconButton}
          >
            <BellDot size={15} strokeWidth={1.75} color={tokens.ink2} />
            {waitingCount > 0 ? <View style={[styles.footerBadge, { backgroundColor: tokens.danger, borderColor: tokens.bg1 }]} /> : null}
          </Pressable>
          <Pressable
            onPress={() => router.push("/history")}
            accessibilityRole="button"
            accessibilityLabel="History"
            style={styles.footerIconButton}
          >
            <History size={15} strokeWidth={1.75} color={tokens.ink2} />
          </Pressable>
          <View style={styles.footerSpacer} />
          <Pressable
            onPress={() => router.push("/settings")}
            accessibilityRole="button"
            accessibilityLabel="Settings"
            style={styles.footerIconButton}
          >
            <Settings2 size={15} strokeWidth={1.75} color={tokens.ink2} />
          </Pressable>
          <Pressable
            onPress={onCollapse}
            accessibilityRole="button"
            accessibilityLabel="Collapse sidebar"
            accessibilityHint="Command Backslash"
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
  footer: { borderTopWidth: StyleSheet.hairlineWidth, paddingTop: space.space8, paddingBottom: space.space4 },
  usageBlock: { paddingHorizontal: space.space4, marginBottom: space.space8 },
  usageLabel: { fontFamily: monoFamily.regular, marginBottom: space.space8 },
  ringRow: { flexDirection: "row", gap: space.space12, alignItems: "center" },
  footerIcons: { flexDirection: "row", alignItems: "center", gap: space.space4, paddingHorizontal: space.space2 },
  footerIconButton: { width: 30, height: 30, alignItems: "center", justifyContent: "center", borderRadius: radii.radius8 },
  footerBadge: { position: "absolute", top: 4, right: 4, width: 6, height: 6, borderRadius: 3, borderWidth: 1 },
  footerSpacer: { flex: 1 },
});
