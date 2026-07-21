// Hearth desktop/web rail (HANDOFF "Fleet + Session" / "Web Fleet"): header + summary,
// top-of-rail TaskComposer (core rule 6), a server section header, session rows (SessionCard
// already carries the Hearth de-boxed/decision-compact treatment), footer icons. This is the
// `master` pane `MasterDetail` (root layout, out of this file's scope) renders at the expanded
// breakpoint on both Tauri desktop and web — same 316px rail shell either way.
import { BellDot, Flame, History, Settings2 } from "lucide-react-native";
import { router, usePathname } from "expo-router";
import React, { useCallback, useMemo, useState } from "react";
import { ScrollView, StyleSheet, Text, View } from "react-native";

import { EmptyState } from "../ds/EmptyState";
import { Button } from "../ds/Button";
import { IconButton } from "../ds/IconButton";
import { TaskComposer } from "../ds/TaskComposer";
import { SessionCard } from "./SessionCard";
import { useAuth } from "../../lib/auth";
import { desktopFleetStatusFromFleet } from "../../lib/connectionHealth";
import { useSessions } from "../../lib/queries";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { formatCost, monoFamily, tabularNums, type as typeScale } from "../../theme/typography";

function RailFooterIcon({ href, icon, label, badge }: { href: "/inbox" | "/history" | "/settings"; icon: React.ReactNode; label: string; badge?: boolean }) {
  const tokens = useTokens();
  const active = usePathname() === href;
  return (
    <IconButton
      icon={icon}
      onPress={() => router.push(href)}
      accessibilityLabel={label}
      badge={badge}
      style={active ? { backgroundColor: tokens.bg3, borderRadius: radii.radius8 } : undefined}
    />
  );
}

export function ExpandedFleetRail() {
  const tokens = useTokens();
  const pathname = usePathname();
  const { servers, activeServerId } = useAuth();
  const sessionsQuery = useSessions();
  const sessions = sessionsQuery.data;
  const fleetStatus = desktopFleetStatusFromFleet(sessionsQuery);
  const rows = useMemo(() => sessions ?? [], [sessions]);
  const waitingCount = useMemo(() => rows.filter((row) => row.waiting).length, [rows]);
  const busyCount = useMemo(() => rows.filter((row) => row.busy).length, [rows]);
  const totalCost = useMemo(() => rows.reduce((sum, row) => sum + row.cost_usd, 0), [rows]);
  const activeServer = servers.find((server) => server.id === activeServerId);
  const selectedSessionId = pathname.match(/^\/session\/([^/]+)/)?.[1];
  const [composerText, setComposerText] = useState("");
  const statusColor =
    fleetStatus.state === "online"
      ? tokens.success
      : fleetStatus.state === "loading"
        ? tokens.accent
        : tokens.danger;
  const onComposerSubmit = useCallback((text: string) => {
    setComposerText("");
    router.push({ pathname: "/new-session", params: { title: text } });
  }, []);

  return (
    <View style={styles.rail}>
      <View style={styles.header}>
        <View style={styles.titleRow}>
          <Text style={[typeScale.headingBold, { color: tokens.ink }]}>Fleet</Text>
          <Text
            style={[styles.mark, { color: tokens.ink3 }]}
            onPress={() => router.push("/floor")}
            accessibilityRole="button"
            accessibilityLabel="Open the floor"
          >
            ⚒
          </Text>
        </View>
        <Text style={[typeScale.sub, { color: tokens.ink3 }]}>
          <Text style={{ color: waitingCount > 0 ? tokens.danger : tokens.ink3 }}>{waitingCount} needs you</Text>
          {` · ${busyCount} forging · `}
          <Text style={[typeScale.monoMeta, tabularNums, { fontFamily: monoFamily.regular }]}>{formatCost(totalCost)}</Text>
          {" today"}
        </Text>
      </View>

      <TaskComposer
        value={composerText}
        onChangeText={setComposerText}
        onSubmit={onComposerSubmit}
        compact
        style={styles.composer}
        testID="rail-composer"
      />
      <ScrollView style={styles.list} contentContainerStyle={styles.listContent}>
        {activeServer ? (
          <View style={styles.serverHeader}>
            <View style={[styles.serverDot, { backgroundColor: statusColor }]} />
            <Text style={[typeScale.section, { color: tokens.ink3 }]}>{activeServer.name}</Text>
            <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.ink4, fontFamily: monoFamily.regular }]}>
              {rows.length}
            </Text>
            <View style={[styles.serverRule, { backgroundColor: tokens.hairline }]} />
          </View>
        ) : null}
        {sessionsQuery.isError && !sessionsQuery.data ? (
          <View style={styles.connectionFailure}>
            <EmptyState icon={Flame} message="Host is offline or unavailable." />
            <Button label="Retry connection" variant="secondary" onPress={() => void sessionsQuery.refetch()} />
          </View>
        ) : sessionsQuery.isLoading && !sessionsQuery.data ? (
          <EmptyState icon={Flame} message="Connecting to host…" />
        ) : rows.length === 0 ? (
          <EmptyState icon={Flame} message="No sessions — forge one above." />
        ) : (
          rows.map((row, index) => <SessionCard key={row.id} row={row} index={index} selected={row.id === selectedSessionId} />)
        )}
      </ScrollView>

      <View style={[styles.footer, { borderTopColor: tokens.border }]}>
        <RailFooterIcon href="/inbox" icon={<BellDot size={16} strokeWidth={1.75} color={tokens.ink2} />} label="Inbox" badge={waitingCount > 0} />
        <RailFooterIcon href="/history" icon={<History size={16} strokeWidth={1.75} color={tokens.ink2} />} label="History" />
        <RailFooterIcon href="/settings" icon={<Settings2 size={16} strokeWidth={1.75} color={tokens.ink2} />} label="Settings" />
        <View style={styles.footerSpacer} />
        <Text style={[typeScale.monoMeta, { color: tokens.ink4, fontFamily: monoFamily.regular }]}>
          {servers.length} server{servers.length === 1 ? "" : "s"} · {fleetStatus.label}
        </Text>
      </View>
    </View>
  );
}

export function DesktopDrillDown({ children }: { children: React.ReactNode }) {
  return children;
}

const styles = StyleSheet.create({
  rail: { flex: 1 },
  header: { paddingHorizontal: space.space20, paddingTop: space.space16, gap: space.space2 },
  titleRow: { flexDirection: "row", alignItems: "center" },
  mark: { fontSize: 13, marginLeft: space.space8, padding: space.space4 },
  composer: { marginHorizontal: space.space12, marginTop: space.space12 },
  list: { flex: 1 },
  listContent: { paddingBottom: space.space16 },
  connectionFailure: { alignItems: "center", gap: space.space12, paddingHorizontal: space.space16 },
  serverHeader: { flexDirection: "row", alignItems: "center", gap: space.space8, paddingHorizontal: space.space16, paddingTop: space.space12, paddingBottom: space.space8 },
  serverDot: { width: 6, height: 6, borderRadius: 3 },
  serverRule: { flex: 1, height: StyleSheet.hairlineWidth },
  footer: { flexDirection: "row", alignItems: "center", gap: space.space2, paddingVertical: space.space8, paddingHorizontal: space.space12, borderTopWidth: StyleSheet.hairlineWidth },
  footerSpacer: { flex: 1 },
});
